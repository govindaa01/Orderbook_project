# hl_mda — Dual-Exchange L2 Order Book MDA

A real-time terminal market data aggregator that streams L2 order book data from
**Hyperliquid** and **Paradex** simultaneously, merges them into a single unified
view, and computes cross-exchange signals — all rendered live in the terminal.

```
┌─ ◈ BTC Merged Order Book   HL ● $67,241.50   PDX ● $67,242.00 ──────────────┐
│                                                                                │
│ ┌─ Merged Book (40%) ──────────┐ ┌─ Signals (20%) ─┐ ┌─ HL ──┐ ┌─ PDX ───┐ │
│ │ ASKS (best at bottom)        │ │ Cross Spread     │ │ ASKS  │ │ ASKS    │ │
│ │ PDX  67244.00  1.2000  ████  │ │   +2.50 (0.004%) │ │ ...   │ │ ...     │ │
│ │ HL   67243.50  0.8000  ███   │ │ Liq Imbalance    │ │ BIDS  │ │ BIDS    │ │
│ │ HL   67242.00  2.1000  █████ │ │   +0.142 ▲ BID   │ │ ...   │ │ ...     │ │
│ ├──────────────────────────────┤ │ [========>  ]    │ └───────┘ └─────────┘ │
│ │ BIDS (best at top)           │ │ Per-Exchange BBO │                        │
│ │ PDX  67241.50  1.5000  ████  │ │ HL  bid/ask ...  │                        │
│ │ HL   67241.00  0.9000  ███   │ │ PDX bid/ask ...  │                        │
│ └──────────────────────────────┘ └──────────────────┘                        │
│ HL: 842 updates   PDX: 317 updates                              [q] Quit      │
└────────────────────────────────────────────────────────────────────────────────┘
```

---

## How to Build and Run

### Prerequisites

- **Rust 1.75+** — install via [rustup](https://rustup.rs): `rustup update stable`
- An internet connection (WebSocket feeds + startup symbol validation)

### Build

```bash
# Clone / unzip the project, then:
cd hl_mda

# Development build
cargo build

# Optimised release build (recommended for actual use)
cargo build --release
```

### Configure

Edit `config.toml` in the project root before running:

```toml
[pair]
hl_symbol  = "BTC"       # Hyperliquid coin (BTC, ETH, SOL, …)
pdx_symbol = "BTC-USD-PERP"  # Paradex market symbol

[display]
depth   = 10    # Merged book depth: 1–10 levels per side
tick_ms = 100   # UI refresh interval in milliseconds (50–2000)
```

### Run

```bash
# Using the config file (default)
cargo run

# Or release binary
./target/release/hl_mda
```

The program validates both symbols against the exchange REST APIs on startup and
exits with a clear error if either is unavailable:

```
Validating symbols against exchanges…
  ✓ Hyperliquid: BTC
  ✓ Paradex: BTC-USD-PERP
Starting feeds…
```

### Controls

| Key | Action |
|-----|--------|
| `q` / `Q` / `Esc` | Quit |

### Logging

Structured logs go to **stderr** so they don't interfere with the TUI on stdout:

```bash
# Pipe logs to a file while watching the TUI
RUST_LOG=debug cargo run 2>mda.log

# Watch the log in another terminal
tail -f mda.log
```

---

## Project Structure

```
hl_mda/
├── config.toml              # Trading pair + display configuration
├── Cargo.toml               # Dependencies
├── README.md
└── src/
    ├── main.rs              # Entry point: config load, validation, TUI loop
    ├── config.rs            # TOML loading, field validation, REST validation
    ├── types.rs             # All shared data types (Level, OrderBook, Exchange, …)
    ├── hyperliquid_mda.rs   # Hyperliquid WebSocket feed (custom binary protocol)
    ├── paradex_mda.rs       # Paradex WebSocket feed (JSON-RPC 2.0 + delta book)
    ├── merger.rs            # Merge two books, compute signals
    └── ui.rs                # ratatui terminal rendering
```

### Architecture

```
main.rs
  │
  ├── config.rs ──────────── load config.toml
  │                           validate HL + PDX symbols via REST (exit on failure)
  │
  ├── hyperliquid_mda.rs ── tokio task ──▶ wss://api.hyperliquid.xyz/ws
  │     reconnect loop                      subscribe l2Book
  │     heartbeat ping/pong (20s)
  │     └── watch::Sender<OrderBook>
  │
  ├── paradex_mda.rs ─────── tokio task ──▶ wss://ws.api.prod.paradex.trade/v1
  │     reconnect loop                       subscribe order_book.*.snapshot@15@100ms
  │     delta-book state (BTreeMap)          heartbeat (20s)
  │     └── watch::Sender<OrderBook>
  │
  └── TUI loop (main thread, 100ms tick)
        ├── hl_rx.borrow()  → OrderBook
        ├── pdx_rx.borrow() → OrderBook
        ├── merger::MergedBook::build() → MergedBook + Signals
        └── ui::draw()
```

### Key design decisions

| Concern | Choice | Rationale |
|---|---|---|
| State sharing | `tokio::sync::watch` | Single-writer, many-reader, zero-copy borrow for rendering; no locking in the UI hot path |
| Paradex book state | `BTreeMap<String, String>` | Prices are keyed lexicographically as strings, so `BTreeMap` gives correct ordering without f64 comparison issues |
| Reconnection | Manual `loop` + `sleep` | Explicit, auditable, no hidden state machine; reconnect delay is configurable |
| Heartbeats | Separate `tokio::spawn` task | Decoupled from the read loop; won't block even if the server is slow |
| Merger | Pure function each tick | No retained merged state; always consistent with the latest snapshot from each exchange |
| Config | TOML file | Human-readable, easy to extend, no CLI flag proliferation |
| Symbol validation | Startup REST call | Fail fast before any WebSocket is opened; lists valid symbols in the error |

---

## Signal Choice and Rationale

### Signal 1 — Cross-Exchange Spread

**Definition:** `best_ask_across_both_exchanges − best_bid_across_both_exchanges`

This is the primary signal for any multi-venue aggregator. A **positive** value
is normal — it represents the cost of crossing. A **negative** value means the
best bid on one exchange is *above* the best ask on another, which is a textbook
arbitrage condition. The UI flags this with a ⚡ ARB badge and switches the
colour to amber so it's immediately visible.

This signal is correct by construction: we pick the globally highest bid and the
globally lowest ask, so it captures the real executable spread.

### Signal 2 — Liquidity Imbalance Ratio (LIR)

**Definition:**

```
LIR = (Σ bid_usd − Σ ask_usd) / (Σ bid_usd + Σ ask_usd)
```

where the sums are over the merged top-10 levels, and each level's USD value is
`price × size`.

**Range:** −1.0 (all liquidity is on the ask side) to +1.0 (all liquidity is on
the bid side).

**Why this signal?**

Order book imbalance is one of the best short-term predictors of price direction
established in the academic literature (Cont, Stoikov & Talreja 2010; Cartea et
al. 2015). The intuition is straightforward: if large resting orders are
concentrated on the bid side, market sell orders will be absorbed quickly and
the price is likely to hold or move up. A heavily ask-skewed book often
precedes downward price movement.

Using **USD notional** (rather than raw contract size) is important across two
exchanges that may have different tick sizes or contract multipliers — it
normalises for those differences and gives a comparable measure.

The UI renders it as a horizontal gauge so a trader can see the direction at a
glance without reading numbers.

**Trade-offs:** The LIR computed here uses the *resting* book only. It doesn't
account for hidden orders, iceberg orders, or the velocity of changes. A more
complete implementation would track how the ratio changes over time.

---

## Assumptions and Trade-offs

### Protocol assumptions

- **Hyperliquid** sends full snapshots on every `l2Book` push. No delta state
  needed client-side.
- **Paradex** sends a snapshot (`update_type: "s"`) on subscribe, followed by
  deltas (`"d"`). The `LocalBook` (`BTreeMap`) in `paradex_mda.rs` maintains the
  running state and materialises the top-N levels on each update.
- Paradex timestamps are in **microseconds**; they are divided by 1000 before
  being stored as milliseconds for consistency with Hyperliquid.

### Ordering

- Paradex prices arrive as strings. The `BTreeMap` key is the raw price string.
  This gives lexicographic ordering, which **does not match numeric ordering**
  for prices with different numbers of decimal places (e.g. `"9.5"` > `"10.0"`
  lexicographically). To handle this correctly the `paradex_mda.rs::to_levels()`
  method parses to `f64` for sorting. If Paradex ever sends malformed price
  strings this will silently produce wrong ordering — a production system would
  validate and log.

### No sequence gap detection

- Paradex sends a `seq_no` field on each update. This implementation does not
  validate sequence continuity. A gap (missed delta) would leave the local book
  in an inconsistent state until the next snapshot. A production system must
  detect gaps and re-subscribe.

### Merged book does not aggregate at the same price

- If both exchanges post a level at exactly the same price, the merged book shows
  them as two separate rows (one tagged HL, one PDX). This is intentional: it
  preserves source attribution and avoids hiding the exchange information.
  A true consolidated tape would sum the sizes, which is a separate design choice.

### Config file path

- `config.toml` is loaded from the **current working directory**, not the binary
  location. Run the binary from the project root, or set an absolute path.

---

## Production Readiness

### What this needs before production

**1. Sequence number / gap detection (Paradex)**
Track `seq_no` and re-subscribe immediately if a gap is detected. Currently a
missed WebSocket frame silently corrupts the book until the next snapshot.

**2. Checksum validation (Hyperliquid)**
Hyperliquid's API supports an optional book checksum. Validating it on each
update guards against data corruption or bugs in the local book state.

**3. Metrics and observability**
Expose Prometheus metrics: update latency, message rates, reconnect counts,
gap counts. Add structured tracing spans around the merge and render pipeline.
Right now all observability is stderr logs.

**4. Latency measurement**
Record the exchange timestamp on each update and compare against wall clock.
Surface the per-exchange data staleness in the UI so traders know if one feed
is lagging.

**5. Proper error classification**
Distinguish transient errors (network blip → reconnect) from fatal errors
(auth failure, symbol delisted → stop and alert). Currently all errors trigger
a reconnect loop.

**6. Configuration hot-reload**
Allow changing `depth` or `tick_ms` without restarting.

**7. Integration / end-to-end tests**
Mock WebSocket servers that replay captured sessions, then assert on the
resulting `OrderBook` state. This is straightforward with `tokio-test` and a
recorded JSON fixture.

**8. Multi-pair support**
The architecture handles exactly one pair. Extending to N pairs would require
`HashMap<Symbol, watch::Sender<OrderBook>>` and per-symbol TUI tabs.

---

## Testing Strategy

### Unit tests (add to each module)

```rust
// merger.rs — property tests
#[test]
fn merged_bids_are_descending() { ... }

#[test]
fn lir_is_zero_when_sides_equal() { ... }

#[test]
fn cross_spread_negative_when_arb_exists() { ... }

// paradex_mda.rs — delta book
#[test]
fn apply_snapshot_clears_old_state() { ... }

#[test]
fn apply_delta_insert_and_delete() { ... }

#[test]
fn gap_in_seq_no_triggers_resubscribe() { ... }

// config.rs
#[test]
fn depth_zero_returns_error() { ... }

#[test]
fn depth_eleven_returns_error() { ... }
```

### Integration tests (mock WS servers)

Use `tokio-tungstenite` in server mode to replay captured JSON sessions.
Assert that after N frames the `OrderBook` matches a known fixture.

```rust
// tests/hl_feed_integration.rs
#[tokio::test]
async fn hl_feed_reconnects_on_close() {
    let server = MockHlServer::new_that_closes_after(3);
    let (tx, rx) = watch::channel(OrderBook::default());
    spawn_hl_feed("BTC".into(), tx);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(rx.borrow().connected);
}
```

### Property-based tests

Use `proptest` or `quickcheck` to fuzz the merger with arbitrary level lists
and assert invariants (bids are always descending, asks always ascending,
LIR is always in −1..=1).

### Manual smoke test

```bash
# Run with debug logging and capture to file
RUST_LOG=debug cargo run 2>smoke.log

# Check for parse errors or unexpected channel names
grep -E "WARN|ERROR" smoke.log
```

---

## What I Would Add with More Time

1. **Sequence gap detection and forced re-subscribe** — the single biggest
   correctness gap right now.

2. **Latency panel** — show exchange timestamp vs. local clock for each feed.
   Useful to know if Paradex is 200ms stale while HL is fresh.

3. **Sparkline history** — a rolling 60-second history of the cross-spread and
   LIR in the signals panel so you can see trends, not just the current value.

4. **Alert thresholds in config** — e.g. `arb_alert_bps = 5` to flash the
   screen if a cross-spread arb exceeds a configurable threshold.

5. **CSV / JSON log output** — write every merged snapshot to a rotating file
   for backtesting signal quality after the fact.

6. **More exchanges** — the `OrderBook` type is exchange-agnostic; adding
   Binance or OKX means adding one new `*_mda.rs` file and one more
   `watch::channel` in `main.rs`. The merger already accepts arbitrary slices.

---

## Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `tokio` | 1 | Async runtime |
| `tokio-tungstenite` | 0.23 | Async WebSocket client with TLS |
| `tungstenite` | 0.23 | WebSocket types |
| `serde` / `serde_json` | 1 | JSON serialisation/deserialisation |
| `toml` | 0.8 | Config file parsing |
| `reqwest` | 0.12 | REST calls for startup symbol validation |
| `ratatui` | 0.28 | Terminal UI framework |
| `crossterm` | 0.28 | Cross-platform terminal control |
| `futures-util` | 0.3 | Async stream combinators (`SinkExt`, `StreamExt`) |
| `anyhow` | 1 | Ergonomic error handling and context chaining |
| `tracing` / `tracing-subscriber` | 0.1/0.3 | Structured logging |
| `chrono` | 0.4 | Timestamp formatting |