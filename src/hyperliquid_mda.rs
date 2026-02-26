// src/hyperliquid_mda.rs — Hyperliquid WebSocket connection manager

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::sync::{watch, Mutex};
use tokio::time::{interval, sleep};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};

use crate::types::{
    Exchange, InboundEnvelope, Level, OrderBook, OutboundMsg, Subscription, WsBook,
};

const HL_WS_URL: &str = "wss://api.hyperliquid.xyz/ws";
const RECONNECT_DELAY_SECS: u64 = 3;
const HEARTBEAT_SECS: u64 = 20;
const MAX_BOOK_DEPTH: usize = 20;

/// Spawns a background task that maintains a live Hyperliquid L2 book.
pub fn spawn_hl_feed(coin: String, book_tx: watch::Sender<OrderBook>) {
    tokio::spawn(async move {
        loop {
            info!("[HL] Connecting…");
            match run_connection(&coin, &book_tx).await {
                Ok(_)  => warn!("[HL] Connection closed cleanly — reconnecting"),
                Err(e) => error!("[HL] Connection error: {e:#} — reconnecting"),
            }
            book_tx.send_modify(|b| b.connected = false);
            sleep(Duration::from_secs(RECONNECT_DELAY_SECS)).await;
        }
    });
}

async fn run_connection(coin: &str, book_tx: &watch::Sender<OrderBook>) -> Result<()> {
    let (ws_stream, _) = connect_async(HL_WS_URL)
        .await
        .context("WebSocket connect failed")?;

    info!("[HL] Connected");
    book_tx.send_modify(|b| {
        b.connected = true;
        b.coin = coin.to_string();
    });

    let (write, mut read) = ws_stream.split();
    let write = Arc::new(Mutex::new(write));

    // Subscribe
    let sub_msg = OutboundMsg::Subscribe {
        subscription: Subscription::L2Book { coin: coin.to_string() },
    };
    {
        let text = serde_json::to_string(&sub_msg)?;
        write.lock().await.send(Message::Text(text)).await?;
        info!("[HL] Subscribed to l2Book:{coin}");
    }

    // Heartbeat task
    let write_clone = Arc::clone(&write);
    let heartbeat = tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(HEARTBEAT_SECS));
        ticker.tick().await;
        loop {
            ticker.tick().await;
            let ping = r#"{"method":"ping"}"#;
            if let Err(e) = write_clone.lock().await.send(Message::Text(ping.to_string())).await {
                error!("[HL] Heartbeat send failed: {e}");
                break;
            }
            debug!("[HL] Sent ping");
        }
    });

    // Message loop
    while let Some(msg) = read.next().await {
        match msg? {
            Message::Text(text) => handle_text(&text, book_tx),
            Message::Close(_)   => { info!("[HL] Server sent close frame"); break; }
            _ => {}
        }
    }

    heartbeat.abort();
    Ok(())
}

fn handle_text(text: &str, book_tx: &watch::Sender<OrderBook>) {
    if text.contains(r#""pong""#) {
        debug!("[HL] Received pong");
        return;
    }

    let env: InboundEnvelope = match serde_json::from_str(text) {
        Ok(v)  => v,
        Err(e) => { warn!("[HL] Parse error: {e} | {text:.200}"); return; }
    };

    match env.channel.as_str() {
        "subscriptionResponse" => debug!("[HL] Subscription confirmed"),
        "l2Book" => {
            if let Ok(book) = parse_l2book(&env.data) {
                book_tx.send_modify(|state| {
                    state.bids = book.levels.0.iter()
                        .take(MAX_BOOK_DEPTH)
                        .map(Level::from_hl)
                        .collect();
                    state.asks = book.levels.1.iter()
                        .take(MAX_BOOK_DEPTH)
                        .map(Level::from_hl)
                        .collect();
                    state.last_update_ms = book.time;
                    state.message_count += 1;
                });
            }
        }
        other => debug!("[HL] Unhandled channel: {other}"),
    }
}

fn parse_l2book(data: &Value) -> Result<WsBook> {
    serde_json::from_value(data.clone()).context("Failed to deserialise WsBook")
}
