// src/config.rs — Load and validate config.toml at startup

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::fs;

const CONFIG_PATH: &str = "config.toml";

// ─── Raw config structs (match config.toml exactly) ──────────────────────────

#[derive(Deserialize, Debug)]
struct RawConfig {
    pair:    RawPair,
    display: RawDisplay,
}

#[derive(Deserialize, Debug)]
struct RawPair {
    hl_symbol:  String,
    pdx_symbol: String,
}

#[derive(Deserialize, Debug)]
struct RawDisplay {
    depth:   usize,
    tick_ms: u64,
}

// ─── Validated config (used by the rest of the app) ──────────────────────────

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub hl_symbol:  String, // e.g. "BTC"
    pub pdx_symbol: String, // e.g. "BTC-USD-PERP"
    pub depth:      usize,  // 1..=10
    pub tick_ms:    u64,    // 50..=1000
}

impl AppConfig {
    /// Load from `config.toml`, validate fields, return error with clear message on failure.
    pub fn load() -> Result<Self> {
        let raw_text = fs::read_to_string(CONFIG_PATH)
            .with_context(|| format!("Cannot read '{CONFIG_PATH}'. Make sure it exists next to the binary."))?;

        let raw: RawConfig = toml::from_str(&raw_text)
            .with_context(|| format!("Failed to parse '{CONFIG_PATH}' as TOML"))?;

        // ── Validate pair fields ──────────────────────────────────────────────
        let hl_symbol = raw.pair.hl_symbol.trim().to_uppercase();
        if hl_symbol.is_empty() {
            bail!("config.toml: pair.hl_symbol must not be empty");
        }

        let pdx_symbol = raw.pair.pdx_symbol.trim().to_uppercase();
        if pdx_symbol.is_empty() {
            bail!("config.toml: pair.pdx_symbol must not be empty");
        }

        // ── Validate display fields ───────────────────────────────────────────
        let depth = raw.display.depth;
        if depth == 0 || depth > 10 {
            bail!("config.toml: display.depth must be between 1 and 10, got {depth}");
        }

        let tick_ms = raw.display.tick_ms;
        if tick_ms < 50 || tick_ms > 2000 {
            bail!("config.toml: display.tick_ms must be between 50 and 2000, got {tick_ms}");
        }

        Ok(AppConfig { hl_symbol, pdx_symbol, depth, tick_ms })
    }
}

// ─── Runtime pair validation ──────────────────────────────────────────────────
//
// We call each exchange's REST API to check the symbol actually exists before
// connecting the WebSocket feeds. Exits with a clear error if unavailable.

/// Validate `hl_symbol` against the Hyperliquid meta endpoint.
/// Returns Ok(()) or an informative error.
pub async fn validate_hl_symbol(symbol: &str) -> Result<()> {
    // HL meta endpoint returns all available perp assets
    let url = "https://api.hyperliquid.xyz/info";
    let body = serde_json::json!({ "type": "meta" });

    let client = reqwest::Client::new();
    let resp = client
        .post(url)
        .json(&body)
        .send()
        .await
        .context("Failed to reach Hyperliquid API for symbol validation")?
        .json::<serde_json::Value>()
        .await
        .context("Failed to parse Hyperliquid meta response")?;

    let universe = resp["universe"]
        .as_array()
        .context("Unexpected Hyperliquid meta response structure")?;

    let known: Vec<&str> = universe
        .iter()
        .filter_map(|v| v["name"].as_str())
        .collect();

    if known.iter().any(|n| n.eq_ignore_ascii_case(symbol)) {
        Ok(())
    } else {
        bail!(
            "Symbol '{}' not found on Hyperliquid.\nAvailable symbols include: {}",
            symbol,
            known.iter().take(10).cloned().collect::<Vec<_>>().join(", ")
        )
    }
}

/// Validate `pdx_symbol` against the Paradex markets REST endpoint.
pub async fn validate_pdx_symbol(symbol: &str) -> Result<()> {
    let url = "https://api.prod.paradex.trade/v1/markets";

    let client = reqwest::Client::new();
    let resp = client
        .get(url)
        .send()
        .await
        .context("Failed to reach Paradex API for symbol validation")?
        .json::<serde_json::Value>()
        .await
        .context("Failed to parse Paradex markets response")?;

    let results = resp["results"]
        .as_array()
        .context("Unexpected Paradex markets response structure")?;

    let known: Vec<&str> = results
        .iter()
        .filter_map(|v| v["symbol"].as_str())
        .collect();

    if known.iter().any(|n| n.eq_ignore_ascii_case(symbol)) {
        Ok(())
    } else {
        bail!(
            "Symbol '{}' not found on Paradex.\nAvailable symbols include: {}",
            symbol,
            known.iter().take(10).cloned().collect::<Vec<_>>().join(", ")
        )
    }
}
