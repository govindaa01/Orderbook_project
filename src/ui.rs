// src/ui.rs — Terminal UI: merged book + individual books + signals panel

use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Gauge, Paragraph, Row, Table},
    Frame,
};

use crate::merger::{MergedBook, MergedLevel};
use crate::types::{Exchange, OrderBook};

// ─── Colour palette ───────────────────────────────────────────────────────────
const C_BID:       Color = Color::Rgb(0,   210, 140); // emerald
const C_ASK:       Color = Color::Rgb(255, 80,  80);  // coral
const C_MID:       Color = Color::Rgb(255, 210, 80);  // amber
const C_DIM:       Color = Color::Rgb(110, 110, 130); // muted
const C_HEADER:    Color = Color::Rgb(160, 160, 220); // lavender
const C_BORDER:    Color = Color::Rgb(55,  55,  90);  // dark indigo
const C_HL:        Color = Color::Rgb(60,  160, 255); // HL blue
const C_PDX:       Color = Color::Rgb(180, 100, 255); // PDX purple
const C_ARB:       Color = Color::Rgb(255, 180, 0);   // arb amber
const C_WARN:      Color = Color::Rgb(255, 60,  60);  // danger red
const C_WHITE:     Color = Color::White;

fn ex_color(ex: &Exchange) -> Color {
    match ex { Exchange::Hyperliquid => C_HL, Exchange::Paradex => C_PDX }
}

fn ex_tag(ex: &Exchange) -> &'static str {
    match ex { Exchange::Hyperliquid => "HL ", Exchange::Paradex => "PDX" }
}

// ─── Public entry point ───────────────────────────────────────────────────────

pub fn draw(frame: &mut Frame, hl: &OrderBook, pdx: &OrderBook, merged: &MergedBook) {
    let area = frame.area();

    // Root: header(3) | body(min) | footer(3)
    let root = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(3),
    ]).split(area);

    draw_header(frame, root[0], hl, pdx);

    // Body: merged book (40%) | signals panel (20%) | HL book (20%) | PDX book (20%)
    let body = Layout::horizontal([
        Constraint::Percentage(40),
        Constraint::Percentage(20),
        Constraint::Percentage(20),
        Constraint::Percentage(20),
    ]).split(root[1]);

    draw_merged_book(frame, body[0], merged);
    draw_signals(frame, body[1], hl, pdx, merged);
    draw_individual_book(frame, body[2], hl);
    draw_individual_book(frame, body[3], pdx);

    draw_footer(frame, root[2], hl, pdx);
}

// ─── Header ───────────────────────────────────────────────────────────────────

fn draw_header(frame: &mut Frame, area: Rect, hl: &OrderBook, pdx: &OrderBook) {
    fn conn(book: &OrderBook, color: Color) -> Vec<Span<'static>> {
        let dot = if book.connected { "●" } else { "○" };
        let dot_color = if book.connected { C_BID } else { C_WARN };
        vec![
            Span::styled(format!("{} ", ex_tag(&book.exchange)), Style::default().fg(color).add_modifier(Modifier::BOLD)),
            Span::styled(dot.to_string(), Style::default().fg(dot_color)),
            Span::styled(
                match book.mid() {
                    Some(m) => format!(" ${m:.2}"),
                    None    => " –".to_string(),
                },
                Style::default().fg(C_MID).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  ", Style::default()),
        ]
    }

    let coin = &hl.coin;
    let mut spans = vec![
        Span::styled(
            format!("  ◈ {coin} Merged Order Book   "),
            Style::default().fg(C_HEADER).add_modifier(Modifier::BOLD),
        ),
    ];
    spans.extend(conn(hl,  C_HL));
    spans.extend(conn(pdx, C_PDX));

    let block = Block::default().borders(Borders::ALL).border_style(Style::default().fg(C_BORDER));
    frame.render_widget(Paragraph::new(Line::from(spans)).block(block), area);
}

// ─── Merged order book ────────────────────────────────────────────────────────

fn draw_merged_book(frame: &mut Frame, area: Rect, merged: &MergedBook) {
    // Split: top half = asks (reversed, best at bottom), bottom half = bids
    let halves = Layout::vertical([
        Constraint::Percentage(50),
        Constraint::Percentage(50),
    ]).split(area);

    draw_merged_side(frame, halves[0], &merged.asks, Side::Ask);
    draw_merged_side(frame, halves[1], &merged.bids, Side::Bid);
}

enum Side { Bid, Ask }

fn draw_merged_side(frame: &mut Frame, area: Rect, levels: &[MergedLevel], side: Side) {
    let (title, price_color, border_color) = match side {
        Side::Bid => ("BIDS", C_BID, C_BID),
        Side::Ask => ("ASKS", C_ASK, C_ASK),
    };

    let max_usd = levels.iter()
        .map(|l| l.price * l.size)
        .fold(0.0_f64, f64::max)
        .max(1.0);

    let header = Row::new([
        Cell::from("Exch").style(Style::default().fg(C_HEADER).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)),
        Cell::from("Price").style(Style::default().fg(C_HEADER).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)),
        Cell::from("Size").style(Style::default().fg(C_HEADER).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)),
        Cell::from("Depth").style(Style::default().fg(C_HEADER).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)),
    ]).height(1);

    // For asks, display bottom-to-top so best ask is closest to the midpoint
    let display_levels: Vec<&MergedLevel> = match side {
        Side::Ask => levels.iter().rev().collect(),
        Side::Bid => levels.iter().collect(),
    };

    let rows: Vec<Row> = display_levels.iter().map(|lvl| {
        let bar_len = ((lvl.price * lvl.size) / max_usd * 14.0).round() as usize;
        let bar = "█".repeat(bar_len);
        let ex_color = ex_color(&lvl.exchange);
        Row::new([
            Cell::from(ex_tag(&lvl.exchange)).style(Style::default().fg(ex_color).add_modifier(Modifier::BOLD)),
            Cell::from(format!("{:.2}", lvl.price)).style(Style::default().fg(price_color).add_modifier(Modifier::BOLD)),
            Cell::from(format!("{:.4}", lvl.size)).style(Style::default().fg(C_WHITE)),
            Cell::from(bar).style(Style::default().fg(price_color)),
        ]).height(1)
    }).collect();

    let widths = [
        Constraint::Length(4),
        Constraint::Length(11),
        Constraint::Length(10),
        Constraint::Min(0),
    ];

    let block = Block::default()
        .title(Span::styled(format!(" {title} "), Style::default().fg(price_color).add_modifier(Modifier::BOLD)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    frame.render_widget(
        Table::new(rows, widths).header(header).block(block)
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
        area,
    );
}

// ─── Signals panel ────────────────────────────────────────────────────────────

fn draw_signals(frame: &mut Frame, area: Rect, hl: &OrderBook, pdx: &OrderBook, merged: &MergedBook) {
    let sig = &merged.signals;

    let rows_area = Layout::vertical([
        Constraint::Length(3),  // panel title block
        Constraint::Length(5),  // cross-exchange spread
        Constraint::Length(1),  // spacer
        Constraint::Length(7),  // liquidity imbalance gauge
        Constraint::Length(1),  // spacer
        Constraint::Length(6),  // per-exchange bbo
        Constraint::Min(0),
    ]).split(area);

    // ── Panel title ───────────────────────────────────────────────────────────
    let title_block = Block::default()
        .title(Span::styled(" ◈ Signals ", Style::default().fg(C_MID).add_modifier(Modifier::BOLD)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(C_BORDER));
    frame.render_widget(title_block, rows_area[0]);

    // ── Cross-exchange spread ─────────────────────────────────────────────────
    let (spread_val, spread_color, arb_label) = match sig.cross_spread {
        Some(s) if s < 0.0 => (
            format!("{s:.4}"),
            C_ARB,
            " ⚡ ARB",
        ),
        Some(s) => (format!("{s:.4}"), C_BID, ""),
        None    => ("–".to_string(), C_DIM, ""),
    };
    let spread_pct = sig.cross_spread_pct
        .map(|p| format!("({p:.4}%)"))
        .unwrap_or_else(|| "–".to_string());

    let best_bid_ex = sig.best_bid_exchange.as_ref()
        .map(|e| Span::styled(format!("Best bid: {} ", ex_tag(e)), Style::default().fg(ex_color(e))))
        .unwrap_or_else(|| Span::raw(""));
    let best_ask_ex = sig.best_ask_exchange.as_ref()
        .map(|e| Span::styled(format!("Best ask: {}", ex_tag(e)), Style::default().fg(ex_color(e))))
        .unwrap_or_else(|| Span::raw(""));

    let spread_lines = vec![
        Line::from(vec![
            Span::styled("Cross Spread  ", Style::default().fg(C_HEADER).add_modifier(Modifier::BOLD)),
            Span::styled(arb_label, Style::default().fg(C_ARB).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled(format!("  {spread_val}  "), Style::default().fg(spread_color).add_modifier(Modifier::BOLD)),
            Span::styled(spread_pct, Style::default().fg(C_DIM)),
        ]),
        Line::from(vec![Span::raw("  "), best_bid_ex]),
        Line::from(vec![Span::raw("  "), best_ask_ex]),
    ];

    let spread_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(C_BORDER));
    frame.render_widget(Paragraph::new(spread_lines).block(spread_block), rows_area[1]);

    // ── Liquidity Imbalance Ratio gauge ───────────────────────────────────────
    // Map -1..+1 → 0..100 for the gauge widget
    let (imb_ratio, imb_label, imb_color) = match sig.liquidity_imbalance {
        Some(r) => {
            let pct = ((r + 1.0) / 2.0 * 100.0) as u16;
            let label = format!("{:+.3}  {}",
                r,
                if r > 0.1 { "▲ BID HEAVY" } else if r < -0.1 { "▼ ASK HEAVY" } else { "≈ BALANCED" }
            );
            let color = if r > 0.2 { C_BID } else if r < -0.2 { C_ASK } else { C_MID };
            (pct, label, color)
        }
        None => (50, "–".to_string(), C_DIM),
    };

    let bid_usd_str  = fmt_usd(sig.total_bid_usd);
    let ask_usd_str  = fmt_usd(sig.total_ask_usd);

    let imb_text = vec![
        Line::from(Span::styled("Liquidity Imbalance", Style::default().fg(C_HEADER).add_modifier(Modifier::BOLD))),
        Line::from(Span::styled(format!("  {imb_label}"), Style::default().fg(imb_color).add_modifier(Modifier::BOLD))),
        Line::from(vec![
            Span::styled(format!("  Bid ${bid_usd_str}"), Style::default().fg(C_BID)),
            Span::styled(format!("  Ask ${ask_usd_str}"), Style::default().fg(C_ASK)),
        ]),
    ];

    let imb_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(C_BORDER));

    // Render text above a gauge
    let imb_inner = Layout::vertical([
        Constraint::Length(4),
        Constraint::Length(1),
    ]).split(rows_area[3]);

    frame.render_widget(Paragraph::new(imb_text).block(imb_block), rows_area[3]);
    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(imb_color).bg(Color::Rgb(30, 30, 50)))
        .percent(imb_ratio.min(100))
        .label(Span::raw(""));
    frame.render_widget(gauge, imb_inner[1]);

    // ── Per-exchange BBO ──────────────────────────────────────────────────────
    let bbo_lines = vec![
        Line::from(Span::styled("Per-Exchange BBO", Style::default().fg(C_HEADER).add_modifier(Modifier::BOLD))),
        Line::from(vec![
            Span::styled("  HL  ", Style::default().fg(C_HL).add_modifier(Modifier::BOLD)),
            Span::styled(
                fmt_bbo(hl.best_bid(), hl.best_ask()),
                Style::default().fg(C_WHITE),
            ),
        ]),
        Line::from(vec![
            Span::styled("  HL  spread: ", Style::default().fg(C_DIM)),
            Span::styled(
                hl.spread().map(|s| format!("{s:.4}")).unwrap_or("–".into()),
                Style::default().fg(C_DIM),
            ),
        ]),
        Line::from(vec![
            Span::styled("  PDX ", Style::default().fg(C_PDX).add_modifier(Modifier::BOLD)),
            Span::styled(
                fmt_bbo(pdx.best_bid(), pdx.best_ask()),
                Style::default().fg(C_WHITE),
            ),
        ]),
        Line::from(vec![
            Span::styled("  PDX spread: ", Style::default().fg(C_DIM)),
            Span::styled(
                pdx.spread().map(|s| format!("{s:.4}")).unwrap_or("–".into()),
                Style::default().fg(C_DIM),
            ),
        ]),
    ];

    let bbo_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(C_BORDER));
    frame.render_widget(Paragraph::new(bbo_lines).block(bbo_block), rows_area[5]);
}

// ─── Individual exchange book (compact) ───────────────────────────────────────

fn draw_individual_book(frame: &mut Frame, area: Rect, book: &OrderBook) {
    let accent = ex_color(&book.exchange);
    let label  = book.exchange.label();
    let conn   = if book.connected { "●" } else { "○" };
    let conn_c = if book.connected { C_BID } else { C_WARN };

    // Split: title(3) | asks(%) | bids(%)
    let parts = Layout::vertical([
        Constraint::Length(3),
        Constraint::Percentage(50),
        Constraint::Percentage(50),
    ]).split(area);

    // Title
    let title_line = Line::from(vec![
        Span::styled(format!(" {label} "), Style::default().fg(accent).add_modifier(Modifier::BOLD)),
        Span::styled(conn, Style::default().fg(conn_c)),
        Span::styled(
            match book.mid() {
                Some(m) => format!(" ${m:.2}"),
                None    => " –".to_string(),
            },
            Style::default().fg(C_MID),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(title_line).block(
            Block::default().borders(Borders::ALL).border_style(Style::default().fg(accent))
        ),
        parts[0],
    );

    draw_indiv_side(frame, parts[1], book, IndivSide::Ask, accent);
    draw_indiv_side(frame, parts[2], book, IndivSide::Bid, accent);
}

enum IndivSide { Bid, Ask }

fn draw_indiv_side(frame: &mut Frame, area: Rect, book: &OrderBook, side: IndivSide, accent: Color) {
    let (levels, title, price_color) = match side {
        IndivSide::Bid => (&book.bids, "BIDS", C_BID),
        IndivSide::Ask => (&book.asks, "ASKS", C_ASK),
    };

    let max_sz = levels.iter().map(|l| l.size_f64()).fold(0.0_f64, f64::max).max(1.0);

    let header = Row::new([
        Cell::from("Price").style(Style::default().fg(C_HEADER).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)),
        Cell::from("Size").style(Style::default().fg(C_HEADER).add_modifier(Modifier::BOLD | Modifier::UNDERLINED)),
        Cell::from("▐").style(Style::default().fg(C_HEADER)),
    ]).height(1);

    let rows: Vec<Row> = levels.iter().map(|lvl| {
        let bar = "█".repeat(((lvl.size_f64() / max_sz) * 6.0).round() as usize);
        Row::new([
            Cell::from(format!("{:.2}", lvl.price_f64())).style(Style::default().fg(price_color).add_modifier(Modifier::BOLD)),
            Cell::from(format!("{:.3}", lvl.size_f64())).style(Style::default().fg(C_WHITE)),
            Cell::from(bar).style(Style::default().fg(price_color)),
        ]).height(1)
    }).collect();

    let block = Block::default()
        .title(Span::styled(format!(" {title} "), Style::default().fg(price_color).add_modifier(Modifier::BOLD)))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(accent));

    let widths = [Constraint::Length(10), Constraint::Length(9), Constraint::Min(0)];
    frame.render_widget(
        Table::new(rows, widths).header(header).block(block)
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
        area,
    );
}

// ─── Footer ───────────────────────────────────────────────────────────────────

fn draw_footer(frame: &mut Frame, area: Rect, hl: &OrderBook, pdx: &OrderBook) {
    let line = Line::from(vec![
        Span::styled(
            format!("  HL: {} updates   PDX: {} updates", hl.message_count, pdx.message_count),
            Style::default().fg(C_DIM),
        ),
        Span::styled(
            format!("{:>width$}", " [q] Quit ", width = area.width.saturating_sub(44) as usize),
            Style::default().fg(C_HEADER),
        ),
    ]);
    let block = Block::default().borders(Borders::ALL).border_style(Style::default().fg(C_BORDER));
    frame.render_widget(Paragraph::new(line).block(block), area);
}

// ─── Format helpers ───────────────────────────────────────────────────────────

fn fmt_usd(v: f64) -> String {
    if v >= 1_000_000.0 { format!("{:.2}M", v / 1_000_000.0) }
    else if v >= 1_000.0 { format!("{:.1}K", v / 1_000.0) }
    else { format!("{v:.2}") }
}

fn fmt_bbo(bid: Option<f64>, ask: Option<f64>) -> String {
    let b = bid.map(|v| format!("{v:.2}")).unwrap_or("–".into());
    let a = ask.map(|v| format!("{v:.2}")).unwrap_or("–".into());
    format!("bid {b}  ask {a}")
}
