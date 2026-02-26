// src/merger.rs — Merge two order books and compute signals

use crate::types::{Exchange, Level, OrderBook};

// ─── Merged level ─────────────────────────────────────────────────────────────

/// A single level in the merged order book, tagged with its source exchange.
#[derive(Debug, Clone)]
pub struct MergedLevel {
    pub price:    f64,
    pub size:     f64,
    pub exchange: Exchange,
}

// ─── Signals ──────────────────────────────────────────────────────────────────

/// Computed signals derived from the two books.
#[derive(Debug, Clone, Default)]
pub struct Signals {
    /// Cross-exchange spread: best ask on one exchange minus best bid on the other.
    /// Negative = arbitrage opportunity exists (bid on one > ask on other).
    pub cross_spread:     Option<f64>,
    pub cross_spread_pct: Option<f64>,

    /// Which side has the better bid and the better ask.
    pub best_bid_exchange: Option<Exchange>,
    pub best_ask_exchange: Option<Exchange>,

    /// Liquidity Imbalance Ratio across the merged top-N book.
    /// = (total_bid_sz - total_ask_sz) / (total_bid_sz + total_ask_sz)
    /// Range: -1.0 (pure ask pressure) to +1.0 (pure bid pressure)
    pub liquidity_imbalance: Option<f64>,

    /// Total bid liquidity in USD (price × size) across top-N levels.
    pub total_bid_usd: f64,
    /// Total ask liquidity in USD across top-N levels.
    pub total_ask_usd: f64,
}

// ─── MergedBook ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct MergedBook {
    pub bids: Vec<MergedLevel>, // top N, descending price
    pub asks: Vec<MergedLevel>, // top N, ascending price
    pub signals: Signals,
}

impl MergedBook {
    /// Build a merged book from two `OrderBook` snapshots, keeping the top `depth` levels.
    pub fn build(hl: &OrderBook, pdx: &OrderBook, depth: usize) -> Self {
        let bids = merge_bids(&hl.bids, &hl.exchange, &pdx.bids, &pdx.exchange, depth);
        let asks = merge_asks(&hl.asks, &hl.exchange, &pdx.asks, &pdx.exchange, depth);
        let signals = compute_signals(hl, pdx, &bids, &asks);
        Self { bids, asks, signals }
    }
}

// ─── Merge helpers ────────────────────────────────────────────────────────────

fn merge_bids(
    a_levels: &[Level], a_ex: &Exchange,
    b_levels: &[Level], b_ex: &Exchange,
    depth: usize,
) -> Vec<MergedLevel> {
    let mut all: Vec<MergedLevel> = a_levels.iter()
        .map(|l| MergedLevel { price: l.price_f64(), size: l.size_f64(), exchange: a_ex.clone() })
        .chain(b_levels.iter()
            .map(|l| MergedLevel { price: l.price_f64(), size: l.size_f64(), exchange: b_ex.clone() }))
        .collect();

    // Bids: highest price first
    all.sort_by(|a, b| b.price.partial_cmp(&a.price).unwrap_or(std::cmp::Ordering::Equal));
    all.truncate(depth);
    all
}

fn merge_asks(
    a_levels: &[Level], a_ex: &Exchange,
    b_levels: &[Level], b_ex: &Exchange,
    depth: usize,
) -> Vec<MergedLevel> {
    let mut all: Vec<MergedLevel> = a_levels.iter()
        .map(|l| MergedLevel { price: l.price_f64(), size: l.size_f64(), exchange: a_ex.clone() })
        .chain(b_levels.iter()
            .map(|l| MergedLevel { price: l.price_f64(), size: l.size_f64(), exchange: b_ex.clone() }))
        .collect();

    // Asks: lowest price first
    all.sort_by(|a, b| a.price.partial_cmp(&b.price).unwrap_or(std::cmp::Ordering::Equal));
    all.truncate(depth);
    all
}

// ─── Signal computation ───────────────────────────────────────────────────────

fn compute_signals(
    hl: &OrderBook,
    pdx: &OrderBook,
    merged_bids: &[MergedLevel],
    merged_asks: &[MergedLevel],
) -> Signals {
    // ── Best bid / ask per exchange ───────────────────────────────────────────
    let hl_best_bid  = hl.best_bid();
    let hl_best_ask  = hl.best_ask();
    let pdx_best_bid = pdx.best_bid();
    let pdx_best_ask = pdx.best_ask();

    // Overall best bid (highest) and ask (lowest) across both exchanges
    let best_bid = max_opt(hl_best_bid, pdx_best_bid);
    let best_ask = min_opt(hl_best_ask, pdx_best_ask);

    let best_bid_exchange = match (hl_best_bid, pdx_best_bid) {
        (Some(h), Some(p)) => Some(if h >= p { Exchange::Hyperliquid } else { Exchange::Paradex }),
        (Some(_), None)    => Some(Exchange::Hyperliquid),
        (None, Some(_))    => Some(Exchange::Paradex),
        _                  => None,
    };

    let best_ask_exchange = match (hl_best_ask, pdx_best_ask) {
        (Some(h), Some(p)) => Some(if h <= p { Exchange::Hyperliquid } else { Exchange::Paradex }),
        (Some(_), None)    => Some(Exchange::Hyperliquid),
        (None, Some(_))    => Some(Exchange::Paradex),
        _                  => None,
    };

    // ── Cross-exchange spread ─────────────────────────────────────────────────
    // Defined as: best_ask - best_bid (negative = arb exists)
    let (cross_spread, cross_spread_pct) = match (best_bid, best_ask) {
        (Some(bid), Some(ask)) => {
            let spread = ask - bid;
            let mid = (bid + ask) / 2.0;
            let pct = if mid > 0.0 { spread / mid * 100.0 } else { 0.0 };
            (Some(spread), Some(pct))
        }
        _ => (None, None),
    };

    // ── Liquidity Imbalance Ratio ─────────────────────────────────────────────
    // (bid_usd - ask_usd) / (bid_usd + ask_usd)  — across merged top-N
    let total_bid_usd: f64 = merged_bids.iter().map(|l| l.price * l.size).sum();
    let total_ask_usd: f64 = merged_asks.iter().map(|l| l.price * l.size).sum();

    let liquidity_imbalance = {
        let total = total_bid_usd + total_ask_usd;
        if total > 0.0 {
            Some((total_bid_usd - total_ask_usd) / total)
        } else {
            None
        }
    };

    Signals {
        cross_spread,
        cross_spread_pct,
        best_bid_exchange,
        best_ask_exchange,
        liquidity_imbalance,
        total_bid_usd,
        total_ask_usd,
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn max_opt(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (Some(x), None)    => Some(x),
        (None, Some(y))    => Some(y),
        _                  => None,
    }
}

fn min_opt(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (Some(x), None)    => Some(x),
        (None, Some(y))    => Some(y),
        _                  => None,
    }
}
