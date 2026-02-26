// src/paradex_mda.rs — Paradex WebSocket connection manager (JSON-RPC 2.0)

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::{watch, Mutex};
use tokio::time::{interval, sleep};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};

use crate::types::{Exchange, Level, OrderBook, PdxBookData, PdxLevel};

const PDX_WS_URL: &str = "wss://ws.api.prod.paradex.trade/v1";
const RECONNECT_DELAY_SECS: u64 = 3;
const HEARTBEAT_SECS: u64 = 20;
const MAX_BOOK_DEPTH: usize = 20;

// ─── JSON-RPC helpers ─────────────────────────────────────────────────────────

/// Build a JSON-RPC 2.0 subscribe message for the order book channel.
/// Channel format: order_book.{market}.snapshot@15@100ms
fn subscribe_msg(market: &str, id: u64) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "subscribe",
        "params": {
            "channel": format!("order_book.{}.snapshot@15@100ms", market)
        },
        "id": id
    })
}

/// Build a JSON-RPC 2.0 heartbeat message (Paradex uses a "heartbeat" method).
fn heartbeat_msg(id: u64) -> Value {
    json!({
        "jsonrpc": "2.0",
        "method": "heartbeat",
        "params": {},
        "id": id
    })
}

// ─── Inbound envelope ────────────────────────────────────────────────────────

/// Generic JSON-RPC 2.0 inbound frame — covers result, error, and subscription push.
#[derive(Deserialize, Debug)]
struct RpcFrame {
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub params: Option<Value>,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<Value>,
    #[serde(default)]
    pub id: Option<u64>,
}

// ─── In-memory book state (for delta maintenance) ────────────────────────────

/// Maintains a local copy of the book so delta updates can be applied.
#[derive(Default)]
struct LocalBook {
    /// price (as ordered string key) → size string
    bids: BTreeMap<String, String>,
    asks: BTreeMap<String, String>,
}

impl LocalBook {
    /// Apply a Paradex snapshot (update_type == "s"): replace everything.
    fn apply_snapshot(&mut self, data: &PdxBookData) {
        self.bids.clear();
        self.asks.clear();
        for lvl in &data.inserts {
            self.apply_insert(lvl);
        }
    }

    /// Apply a Paradex delta (update_type == "d").
    fn apply_delta(&mut self, data: &PdxBookData) {
        for lvl in &data.deletes {
            self.remove(lvl);
        }
        for lvl in &data.updates {
            self.apply_insert(lvl);
        }
        for lvl in &data.inserts {
            self.apply_insert(lvl);
        }
    }

    fn apply_insert(&mut self, lvl: &PdxLevel) {
        let map = if lvl.side == "BUY" { &mut self.bids } else { &mut self.asks };
        map.insert(lvl.price.clone(), lvl.size.clone());
    }

    fn remove(&mut self, lvl: &PdxLevel) {
        let map = if lvl.side == "BUY" { &mut self.bids } else { &mut self.asks };
        map.remove(&lvl.price);
    }

    /// Materialise the top N bids (descending price) and asks (ascending price).
    fn to_levels(&self, depth: usize) -> (Vec<Level>, Vec<Level>) {
        // Bids: highest price first
        let bids = self.bids.iter().rev().take(depth)
            .map(|(px, sz)| Level { price: px.clone(), size: sz.clone(), count: 0 })
            .collect();

        // Asks: lowest price first
        let asks = self.asks.iter().take(depth)
            .map(|(px, sz)| Level { price: px.clone(), size: sz.clone(), count: 0 })
            .collect();

        (bids, asks)
    }
}

// ─── Public entry point ───────────────────────────────────────────────────────

/// Spawns a background task that maintains a live Paradex L2 book.
/// `market` should be the Paradex market symbol e.g. "BTC-USD-PERP".
pub fn spawn_pdx_feed(market: String, book_tx: watch::Sender<OrderBook>) {
    tokio::spawn(async move {
        loop {
            info!("[PDX] Connecting…");
            match run_connection(&market, &book_tx).await {
                Ok(_)  => warn!("[PDX] Connection closed cleanly — reconnecting"),
                Err(e) => error!("[PDX] Connection error: {e:#} — reconnecting"),
            }
            book_tx.send_modify(|b| b.connected = false);
            sleep(Duration::from_secs(RECONNECT_DELAY_SECS)).await;
        }
    });
}

async fn run_connection(market: &str, book_tx: &watch::Sender<OrderBook>) -> Result<()> {
    let (ws_stream, _) = connect_async(PDX_WS_URL)
        .await
        .context("WebSocket connect failed")?;

    info!("[PDX] Connected");
    book_tx.send_modify(|b| {
        b.connected = true;
        b.coin = market.to_string();
    });

    let (write, mut read) = ws_stream.split();
    let write = Arc::new(Mutex::new(write));

    // Subscribe to snapshot feed
    {
        let msg = serde_json::to_string(&subscribe_msg(market, 1))?;
        write.lock().await.send(Message::Text(msg)).await?;
        info!("[PDX] Subscribed to order_book.{market}.snapshot@15@100ms");
    }

    // Heartbeat task
    let write_clone = Arc::clone(&write);
    let heartbeat = tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(HEARTBEAT_SECS));
        let mut hb_id: u64 = 100;
        ticker.tick().await; // skip immediate first tick
        loop {
            ticker.tick().await;
            let msg = match serde_json::to_string(&heartbeat_msg(hb_id)) {
                Ok(s) => s,
                Err(e) => { error!("[PDX] Failed to serialise heartbeat: {e}"); break; }
            };
            if let Err(e) = write_clone.lock().await.send(Message::Text(msg)).await {
                error!("[PDX] Heartbeat send failed: {e}");
                break;
            }
            debug!("[PDX] Sent heartbeat id={hb_id}");
            hb_id += 1;
        }
    });

    // Local book state — lives for the duration of this connection
    let mut local_book = LocalBook::default();

    // Message loop
    while let Some(msg) = read.next().await {
        match msg? {
            Message::Text(text) => handle_text(&text, &mut local_book, book_tx),
            Message::Close(_)   => { info!("[PDX] Server sent close frame"); break; }
            _ => {}
        }
    }

    heartbeat.abort();
    Ok(())
}

fn handle_text(text: &str, local_book: &mut LocalBook, book_tx: &watch::Sender<OrderBook>) {
    let frame: RpcFrame = match serde_json::from_str(text) {
        Ok(f)  => f,
        Err(e) => { warn!("[PDX] Parse error: {e} | {text:.200}"); return; }
    };

    // JSON-RPC error
    if let Some(err) = &frame.error {
        warn!("[PDX] RPC error: {err}");
        return;
    }

    // Subscribe/heartbeat result ack
    if frame.result.is_some() {
        debug!("[PDX] RPC ack id={:?}", frame.id);
        return;
    }

    // Subscription push: method == "subscription"
    if frame.method.as_deref() == Some("subscription") {
        let params = match &frame.params {
            Some(p) => p,
            None    => { warn!("[PDX] subscription push with no params"); return; }
        };

        let data_val = match params.get("data") {
            Some(d) => d,
            None    => { warn!("[PDX] subscription push with no data field"); return; }
        };

        let data: PdxBookData = match serde_json::from_value(data_val.clone()) {
            Ok(d)  => d,
            Err(e) => { warn!("[PDX] Failed to parse PdxBookData: {e}"); return; }
        };

        // Apply to local book
        match data.update_type.as_str() {
            "s" => local_book.apply_snapshot(&data),
            "d" => local_book.apply_delta(&data),
            ut  => { debug!("[PDX] Unknown update_type: {ut}"); return; }
        }

        // Materialise and push to watch channel
        let (bids, asks) = local_book.to_levels(MAX_BOOK_DEPTH);
        book_tx.send_modify(|state| {
            state.bids = bids;
            state.asks = asks;
            state.last_update_ms = data.last_updated_at / 1_000; // Paradex uses microseconds
            state.message_count += 1;
        });
    }
}
