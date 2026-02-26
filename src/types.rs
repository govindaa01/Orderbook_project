// src/types.rs — Shared data types for all exchange feeds

use serde::{Deserialize, Serialize};

// ─── Hyperliquid outbound messages ───────────────────────────────────────────

#[derive(Serialize, Debug, Clone)]
#[serde(tag = "method", rename_all = "camelCase")]
pub enum OutboundMsg {
    Subscribe {
        subscription: Subscription,
    },
    Unsubscribe {
        subscription: Subscription,
    },
    #[serde(rename = "ping")]
    Ping {},
}

#[derive(Serialize, Debug, Clone)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Subscription {
    #[serde(rename = "l2Book")]
    L2Book { coin: String },
}

// ─── Hyperliquid inbound messages ────────────────────────────────────────────

/// Top-level envelope from the Hyperliquid server.
#[derive(Deserialize, Debug)]
pub struct InboundEnvelope {
    pub channel: String,
    pub data: serde_json::Value,
}

/// Parsed Hyperliquid l2Book update.
#[derive(Deserialize, Debug, Clone)]
pub struct WsBook {
    pub coin: String,
    pub levels: (Vec<WsLevel>, Vec<WsLevel>), // (bids, asks)
    pub time: u64,
}

/// A single Hyperliquid price level.
#[derive(Deserialize, Debug, Clone)]
pub struct WsLevel {
    pub px: String,
    pub sz: String,
    pub n: u32,
}

impl WsLevel {
    pub fn price_f64(&self) -> f64 { self.px.parse().unwrap_or(0.0) }
    pub fn size_f64(&self)  -> f64 { self.sz.parse().unwrap_or(0.0) }
}

// ─── Paradex inbound messages ────────────────────────────────────────────────

/// A single Paradex order book level (snapshot + delta messages).
#[derive(Deserialize, Debug, Clone)]
pub struct PdxLevel {
    pub price: String,
    pub side: String, // "BUY" | "SELL"
    pub size: String,
}

impl PdxLevel {
    pub fn price_f64(&self) -> f64 { self.price.parse().unwrap_or(0.0) }
    pub fn size_f64(&self)  -> f64 { self.size.parse().unwrap_or(0.0) }
}

/// The `data` payload inside a Paradex `subscription` push.
#[derive(Deserialize, Debug, Clone)]
pub struct PdxBookData {
    pub inserts: Vec<PdxLevel>,
    pub deletes: Vec<PdxLevel>,
    pub updates: Vec<PdxLevel>,
    pub last_updated_at: u64,
    pub market: String,
    /// "s" = snapshot, "d" = delta
    pub update_type: String,
}

// ─── Normalised price level (shared by both exchanges) ───────────────────────

/// Canonical price level stored in `OrderBook`.
#[derive(Debug, Clone)]
pub struct Level {
    pub price: String,
    pub size: String,
    pub count: u32, // Paradex doesn't provide order count → 0
}

impl Level {
    pub fn price_f64(&self) -> f64 { self.price.parse().unwrap_or(0.0) }
    pub fn size_f64(&self)  -> f64 { self.size.parse().unwrap_or(0.0) }

    pub fn from_hl(l: &WsLevel) -> Self {
        Self { price: l.px.clone(), size: l.sz.clone(), count: l.n }
    }

    pub fn from_pdx(l: &PdxLevel) -> Self {
        Self { price: l.price.clone(), size: l.size.clone(), count: 0 }
    }
}

// ─── Exchange label ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq)]
pub enum Exchange {
    #[default]
    Hyperliquid,
    Paradex,
}

impl Exchange {
    pub fn label(&self) -> &'static str {
        match self {
            Exchange::Hyperliquid => "Hyperliquid",
            Exchange::Paradex     => "Paradex",
        }
    }
    pub fn short(&self) -> &'static str {
        match self {
            Exchange::Hyperliquid => "HL",
            Exchange::Paradex     => "PDX",
        }
    }
}

// ─── Normalised order book ───────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct OrderBook {
    pub exchange: Exchange,
    pub coin: String,
    pub bids: Vec<Level>,
    pub asks: Vec<Level>,
    pub last_update_ms: u64,
    pub connected: bool,
    pub message_count: u64,
}

impl OrderBook {
    pub fn new(exchange: Exchange, coin: &str) -> Self {
        Self {
            exchange,
            coin: coin.to_string(),
            ..Default::default()
        }
    }

    pub fn best_bid(&self) -> Option<f64> { self.bids.first().map(|l| l.price_f64()) }
    pub fn best_ask(&self) -> Option<f64> { self.asks.first().map(|l| l.price_f64()) }

    pub fn mid(&self) -> Option<f64> {
        match (self.best_bid(), self.best_ask()) {
            (Some(b), Some(a)) => Some((b + a) / 2.0),
            _ => None,
        }
    }

    pub fn spread(&self) -> Option<f64> {
        match (self.best_bid(), self.best_ask()) {
            (Some(b), Some(a)) => Some(a - b),
            _ => None,
        }
    }

    pub fn spread_pct(&self) -> Option<f64> {
        match (self.spread(), self.mid()) {
            (Some(s), Some(m)) if m > 0.0 => Some(s / m * 100.0),
            _ => None,
        }
    }
}
