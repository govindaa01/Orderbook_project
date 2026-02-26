// src/main.rs — Dual-exchange L2 MDA entry point

mod config;
mod hyperliquid_mda;
mod merger;
mod paradex_mda;
mod types;
mod ui;

use std::io;
use std::time::Duration;

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use tokio::sync::watch;
use tracing::info;
use tracing_subscriber::{fmt, EnvFilter};

use crate::config::AppConfig;
use crate::merger::MergedBook;
use crate::types::{Exchange, OrderBook};

// ─── Entry point ─────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    // Logging to stderr so it doesn't interfere with the TUI on stdout
    fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(io::stderr)
        .init();

    // ── Load and validate config ──────────────────────────────────────────────
    let cfg = AppConfig::load().unwrap_or_else(|e| {
        eprintln!("\n❌  Configuration error:\n    {e}\n");
        std::process::exit(1);
    });

    info!("Config loaded: HL={} PDX={}", cfg.hl_symbol, cfg.pdx_symbol);

    // Validate symbols against both exchanges before connecting WebSockets
    eprintln!("Validating symbols against exchanges…");

    if let Err(e) = config::validate_hl_symbol(&cfg.hl_symbol).await {
        eprintln!("\n❌  Hyperliquid symbol validation failed:\n    {e}\n");
        std::process::exit(1);
    }
    eprintln!("  ✓ Hyperliquid: {}", cfg.hl_symbol);

    if let Err(e) = config::validate_pdx_symbol(&cfg.pdx_symbol).await {
        eprintln!("\n❌  Paradex symbol validation failed:\n    {e}\n");
        std::process::exit(1);
    }
    eprintln!("  ✓ Paradex: {}", cfg.pdx_symbol);
    eprintln!("Starting feeds…\n");

    // ── Shared state channels ─────────────────────────────────────────────────
    let (hl_tx, hl_rx)   = watch::channel(OrderBook::new(Exchange::Hyperliquid, &cfg.hl_symbol));
    let (pdx_tx, pdx_rx) = watch::channel(OrderBook::new(Exchange::Paradex, &cfg.pdx_symbol));

    // ── Spawn exchange feeds ──────────────────────────────────────────────────
    hyperliquid_mda::spawn_hl_feed(cfg.hl_symbol.clone(), hl_tx);
    paradex_mda::spawn_pdx_feed(cfg.pdx_symbol.clone(), pdx_tx);

    // ── Run TUI ───────────────────────────────────────────────────────────────
    run_tui(hl_rx, pdx_rx, cfg).await?;

    Ok(())
}

// ─── TUI loop ────────────────────────────────────────────────────────────────

async fn run_tui(
    mut hl_rx:  watch::Receiver<OrderBook>,
    mut pdx_rx: watch::Receiver<OrderBook>,
    cfg: AppConfig,
) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let tick = Duration::from_millis(cfg.tick_ms);

    'main: loop {
        let hl_book  = hl_rx.borrow_and_update().clone();
        let pdx_book = pdx_rx.borrow_and_update().clone();
        let merged   = MergedBook::build(&hl_book, &pdx_book, cfg.depth);

        terminal.draw(|f| ui::draw(f, &hl_book, &pdx_book, &merged))?;

        if event::poll(tick)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => break 'main,
                        _ => {}
                    }
                }
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    info!("Goodbye!");
    Ok(())
}