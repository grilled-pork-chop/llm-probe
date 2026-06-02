//! Live `htop`/`btop`-style dashboard (`--tui`, `#[cfg(feature = "tui")]`).
//!
//! The TUI consumes the *same* `RunEvent`/`RequestOutcome` stream as plain mode
//! (§9), so the numbers cannot diverge. Terminal setup is wrapped in a RAII
//! guard whose `Drop` restores the screen; a panic hook restores first too, so a
//! crash or `Ctrl-C` never leaves the shell wrecked.

use crate::config::RunConfig;
use crate::error::ProbeError;
use crate::metrics::{Report, RequestOutcome, aggregate};
use crate::runner::{BatchResult, RunEvent, run_batch};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    BarChart, Block, BorderType, Borders, Cell, Clear, Gauge, Padding, Paragraph, Row, Sparkline,
    Table, TableState, Wrap,
};
use std::collections::{BTreeMap, VecDeque};
use std::io::{self};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::unbounded_channel;
use tokio::sync::watch;

const ACCENT: Color = Color::Cyan;
const TICK: Duration = Duration::from_millis(80);
const SPARK_LEN: usize = 60;
const MAX_ROWS: usize = 200;
/// Most recent request rows kept for the list/inspector (bounds memory when
/// running indefinitely; far exceeds any sane concurrency so in-flight rows
/// are never evicted before they finish).
const KEEP_ROWS: usize = 1_000;
/// Most recent outcomes kept for the live stats window when running forever.
const KEEP_OUTCOMES: usize = 5_000;

/// Restores the terminal on drop (normal return *or* unwind).
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> Result<Self, ProbeError> {
        enable_raw_mode().map_err(io_err)?;
        execute!(io::stdout(), EnterAlternateScreen).map_err(io_err)?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

fn io_err(e: io::Error) -> ProbeError {
    ProbeError::Stream(format!("terminal error: {e}"))
}

/// Run the dashboard to completion (or until the user quits) and return the
/// batch outcomes so the caller can print the §8 report after restoring.
pub async fn run(cfg: &RunConfig) -> Result<BatchResult, ProbeError> {
    // Restore the terminal before re-raising any panic.
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        prev_hook(info);
    }));

    let _guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend).map_err(io_err)?;

    // Spawn the batch; it streams events back over `run_rx`. `pause_tx` gates
    // dispatch so `space`/`p` can pause sending new requests.
    let (run_tx, mut run_rx) = unbounded_channel::<RunEvent>();
    let (pause_tx, pause_rx) = watch::channel(false);
    let cfg_task = cfg.clone();
    let mut batch =
        tokio::spawn(async move { run_batch(&cfg_task, Some(run_tx), Some(pause_rx)).await });

    // Blocking key reader on its own thread (A.14): no `event-stream` feature.
    let (key_tx, mut key_rx) = unbounded_channel::<Event>();
    std::thread::spawn(move || {
        while let Ok(ev) = event::read() {
            if key_tx.send(ev).is_err() {
                break;
            }
        }
    });

    let mut state = TuiState::new(cfg.requests);
    let mut ticker = tokio::time::interval(TICK);
    // The runner's authoritative result (correct wall-clock), captured when the
    // batch task finishes. `None` until then.
    let mut batch_result: Option<Result<BatchResult, ProbeError>> = None;

    // The loop exits only when the user quits (q / Esc / Ctrl-C). On completion
    // the dashboard stays up showing the frozen final frame (§9), so a fast or
    // instantly-failing run is still visible instead of flashing past.
    loop {
        tokio::select! {
            res = &mut batch, if batch_result.is_none() => {
                batch_result = Some(match res {
                    Ok(inner) => inner,
                    Err(e) => Err(ProbeError::Stream(format!("batch task failed: {e}"))),
                });
            }
            Some(ev) = run_rx.recv() => state.apply(ev),
            Some(ev) = key_rx.recv() => {
                if let Event::Key(k) = ev
                    && k.kind == KeyEventKind::Press
                {
                    // Ctrl-C reaches us as a key in raw mode (not a signal); quit on it.
                    let ctrl_c =
                        k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL);
                    if ctrl_c || handle_key(&mut state, k.code, &pause_tx) {
                        break;
                    }
                }
            }
            _ = ticker.tick() => {
                state.on_tick();
                terminal.draw(|f| draw(f, &state, cfg)).map_err(io_err)?;
            }
        }
    }

    // Prefer the runner's authoritative result; fall back to what we collected
    // if the user quit before the batch finished (the task is then abandoned).
    match batch_result {
        Some(Ok(result)) => Ok(result),
        Some(Err(e)) => Err(e),
        None => {
            batch.abort();
            Ok(BatchResult {
                outcomes: Vec::from(std::mem::take(&mut state.outcomes)),
                wall_clock: state.elapsed(),
            })
        }
    }
}

/// Returns `true` when the key requests quit. Overlays capture keys first.
/// `pause_tx` gates request dispatch in the runner.
fn handle_key(state: &mut TuiState, code: KeyCode, pause_tx: &watch::Sender<bool>) -> bool {
    // Help overlay: any of ? / Esc / q closes it.
    if state.show_help {
        if matches!(code, KeyCode::Char('?') | KeyCode::Esc | KeyCode::Char('q')) {
            state.show_help = false;
        }
        return false;
    }
    // Detail overlay: scroll within it; Enter/Esc close it.
    if state.show_detail {
        match code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => state.show_detail = false,
            KeyCode::Up | KeyCode::Char('k') => {
                state.detail_scroll = state.detail_scroll.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                state.detail_scroll = state.detail_scroll.saturating_add(1);
            }
            KeyCode::Char('g') => state.detail_scroll = 0,
            _ => {}
        }
        return false;
    }

    let last = state.started.saturating_sub(1);
    match code {
        KeyCode::Char('q') | KeyCode::Esc => return true,
        // Pause/resume sending new requests and freeze the live view.
        KeyCode::Char('p') | KeyCode::Char(' ') => {
            state.toggle_pause();
            let _ = pause_tx.send(state.paused);
        }
        KeyCode::Char('?') => state.show_help = true,
        // Open the inspector for the selected request.
        KeyCode::Enter => {
            if state.started > 0 {
                state.show_detail = true;
                state.detail_scroll = 0;
            }
        }
        KeyCode::Up | KeyCode::Char('k') => state.selected = state.selected.saturating_sub(1),
        KeyCode::Down | KeyCode::Char('j') => state.selected = (state.selected + 1).min(last),
        KeyCode::Char('g') => state.selected = 0,
        KeyCode::Char('G') => state.selected = last,
        _ => {}
    }
    false
}

/// One displayed request row.
#[derive(Clone)]
struct RowEntry {
    id: usize,
    /// `None` = in flight; `Some(outcome)` = finished.
    outcome: Option<RequestOutcome>,
    ttft: Option<Duration>,
}

/// In-memory dashboard state, mutated by events and read by the renderer.
struct TuiState {
    start: Instant,
    total: usize,
    started: usize,
    finished: usize,
    ok: usize,
    failed: usize,
    in_flight: usize,
    done: bool,
    /// Paused: dispatch is gated and the live view is frozen.
    paused: bool,
    /// Total time spent paused, excluded from rate denominators.
    paused_total: Duration,
    /// Start of the current pause, if any.
    pause_start: Option<Instant>,
    show_help: bool,
    /// Inspector overlay for the selected request.
    show_detail: bool,
    /// Selected row index into the newest-first request list.
    selected: usize,
    /// Vertical scroll within the open detail overlay.
    detail_scroll: u16,
    /// Rolling window of recent outcomes for live stats.
    outcomes: VecDeque<RequestOutcome>,
    /// Recent request rows by id (newest kept; oldest evicted past `KEEP_ROWS`).
    rows: BTreeMap<usize, RowEntry>,
    report: Option<Report>,
    tps_hist: VecDeque<u64>,
    ttft_hist: VecDeque<u64>,
    /// Set once the batch completes; freezes rate metrics (see `elapsed`).
    final_elapsed: Option<Duration>,
}

impl TuiState {
    /// `total == 0` means the run is unbounded (runs until the user quits).
    fn new(total: usize) -> Self {
        Self {
            start: Instant::now(),
            total,
            started: 0,
            finished: 0,
            ok: 0,
            failed: 0,
            in_flight: 0,
            done: false,
            paused: false,
            paused_total: Duration::ZERO,
            pause_start: None,
            show_help: false,
            show_detail: false,
            selected: 0,
            detail_scroll: 0,
            outcomes: VecDeque::new(),
            rows: BTreeMap::new(),
            report: None,
            tps_hist: VecDeque::with_capacity(SPARK_LEN),
            ttft_hist: VecDeque::with_capacity(SPARK_LEN),
            final_elapsed: None,
        }
    }

    /// True when running indefinitely (`-n 0`).
    fn infinite(&self) -> bool {
        self.total == 0
    }

    /// Existing rows, newest id first — the order shown in the table and the
    /// order `selected` indexes into.
    fn sorted_rows(&self) -> Vec<&RowEntry> {
        self.rows.values().rev().collect()
    }

    /// Wall-clock used for rate metrics. Frozen at completion, and excludes time
    /// spent paused — so aggregate TPS / req/s / the timer hold steady while
    /// paused and resume cleanly afterwards.
    fn elapsed(&self) -> Duration {
        if let Some(done) = self.final_elapsed {
            return done;
        }
        let paused = self.paused_total + self.pause_start.map(|t| t.elapsed()).unwrap_or_default();
        self.start.elapsed().saturating_sub(paused)
    }

    /// Toggle pause: stop/resume the elapsed clock used for rates.
    fn toggle_pause(&mut self) {
        if self.paused {
            if let Some(started) = self.pause_start.take() {
                self.paused_total += started.elapsed();
            }
            self.paused = false;
        } else {
            self.pause_start = Some(Instant::now());
            self.paused = true;
        }
    }

    fn apply(&mut self, ev: RunEvent) {
        match ev {
            RunEvent::Started { id } => {
                self.started += 1;
                self.in_flight += 1;
                self.rows.insert(
                    id,
                    RowEntry {
                        id,
                        outcome: None,
                        ttft: None,
                    },
                );
                self.prune_rows();
            }
            RunEvent::FirstToken { id, ttft } => {
                if let Some(row) = self.rows.get_mut(&id) {
                    row.ttft = Some(ttft);
                }
            }
            RunEvent::Finished { id, outcome } => {
                self.finished += 1;
                self.in_flight = self.in_flight.saturating_sub(1);
                if outcome.success {
                    self.ok += 1;
                } else {
                    self.failed += 1;
                }
                self.rows.insert(
                    id,
                    RowEntry {
                        id,
                        ttft: outcome.ttft,
                        outcome: Some(outcome.clone()),
                    },
                );
                self.prune_rows();

                self.outcomes.push_back(outcome);
                if self.outcomes.len() > KEEP_OUTCOMES {
                    self.outcomes.pop_front();
                }

                // Only finite runs complete; an infinite run keeps going.
                if !self.infinite() && self.finished >= self.total && !self.done {
                    self.done = true;
                    self.final_elapsed = Some(self.start.elapsed());
                }
            }
        }
    }

    /// Drop the oldest rows once the window is full (no-op for typical finite runs).
    fn prune_rows(&mut self) {
        while self.rows.len() > KEEP_ROWS {
            if let Some((&oldest, _)) = self.rows.iter().next() {
                self.rows.remove(&oldest);
            } else {
                break;
            }
        }
    }

    /// Recompute live stats and push sparkline samples (decoupled from event
    /// ingestion → no flicker).
    fn on_tick(&mut self) {
        // While paused, hold the last snapshot so the whole view is readable.
        if self.paused {
            return;
        }
        // Frozen elapsed once done, so the aggregate stops moving on the final frame.
        let elapsed = self.elapsed();
        let report = aggregate(self.outcomes.make_contiguous(), elapsed);
        // Only extend the sparklines while the batch is live; a completed run
        // keeps its history rather than scrolling a flat tail forever.
        if !self.done {
            push_capped(
                &mut self.tps_hist,
                report.aggregate_tps.unwrap_or(0.0).round() as u64,
            );
            let ttft_ms = report
                .ttft
                .map(|t| (t.avg * 1000.0).round() as u64)
                .unwrap_or(0);
            push_capped(&mut self.ttft_hist, ttft_ms);
        }
        self.report = Some(report);
    }
}

fn push_capped(buf: &mut VecDeque<u64>, v: u64) {
    if buf.len() == SPARK_LEN {
        buf.pop_front();
    }
    buf.push_back(v);
}

// ---- rendering ------------------------------------------------------------

fn draw(frame: &mut ratatui::Frame, state: &TuiState, cfg: &RunConfig) {
    let areas = Layout::vertical([
        Constraint::Length(3), // header
        Constraint::Length(4), // progress
        Constraint::Length(6), // metric tiles
        Constraint::Length(6), // sparklines
        Constraint::Length(9), // distribution
        Constraint::Min(4),    // request table
        Constraint::Length(1), // footer
    ])
    .split(frame.area());

    draw_header(frame, areas[0], state, cfg);
    draw_progress(frame, areas[1], state);
    draw_tiles(frame, areas[2], state);
    draw_sparklines(frame, areas[3], state);
    draw_distribution(frame, areas[4], state);
    draw_requests(frame, areas[5], state);
    draw_footer(frame, areas[6], state);

    if state.show_detail {
        draw_detail(frame, state, cfg);
    }
    if state.show_help {
        draw_help(frame);
    }
}

fn panel(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ))
}

fn draw_header(frame: &mut ratatui::Frame, area: Rect, state: &TuiState, cfg: &RunConfig) {
    let mode = if cfg.stream { "streaming" } else { "blocking" };
    let elapsed = state.elapsed().as_secs_f64();
    let line = Line::from(vec![
        Span::raw(format!(
            "{} · {} · {mode} · c={}",
            cfg.endpoint, cfg.model, cfg.concurrency
        )),
        Span::raw("    "),
        Span::styled(format!("⏱ {elapsed:5.1}s"), Style::default().fg(ACCENT)),
    ]);
    frame.render_widget(Paragraph::new(line).block(panel("llmprobe")), area);
}

fn draw_progress(frame: &mut ratatui::Frame, area: Rect, state: &TuiState) {
    let rows = Layout::vertical([Constraint::Length(1), Constraint::Length(1)])
        .margin(1)
        .split(area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded);
    frame.render_widget(block, area);

    if state.infinite() {
        // No known total: show a steady full bar with running counters.
        let gauge = Gauge::default()
            .gauge_style(Style::default().fg(ACCENT))
            .ratio(1.0)
            .label(format!(
                "∞ running   {} done · {} live · ✓ {} · ✗ {}",
                state.finished, state.in_flight, state.ok, state.failed
            ));
        frame.render_widget(gauge, rows[0]);
    } else {
        let ratio = if state.total == 0 {
            0.0
        } else {
            state.finished as f64 / state.total as f64
        };
        let gauge = Gauge::default()
            .gauge_style(Style::default().fg(gauge_color(ratio)))
            .ratio(ratio)
            .label(format!(
                "{:.0}%   {}/{} · {} live · ✓ {} · ✗ {}",
                ratio * 100.0,
                state.finished,
                state.total,
                state.in_flight,
                state.ok,
                state.failed
            ));
        frame.render_widget(gauge, rows[0]);
    }

    let success = if state.finished == 0 {
        100.0
    } else {
        state.ok as f64 / state.finished as f64 * 100.0
    };
    let elapsed = state.elapsed().as_secs_f64();
    let rps = if elapsed > 0.0 {
        state.ok as f64 / elapsed
    } else {
        0.0
    };
    frame.render_widget(
        Paragraph::new(format!("success {success:.1}%    req/s {rps:.1}")),
        rows[1],
    );
}

fn draw_tiles(frame: &mut ratatui::Frame, area: Rect, state: &TuiState) {
    let cols = Layout::horizontal([
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
        Constraint::Ratio(1, 3),
    ])
    .split(area);
    let r = state.report.as_ref();

    let throughput = vec![
        tile_line(opt(r.and_then(|r| r.aggregate_tps), "tok/s"), "aggregate"),
        tile_line(opt(r.and_then(|r| r.avg_tps), "tok/s"), "per req"),
        tile_line(opt(r.and_then(|r| r.mean_itl_ms), "ms"), "inter-token"),
    ];
    frame.render_widget(
        Paragraph::new(throughput).block(panel("throughput")),
        cols[0],
    );

    let e2e = r.and_then(|r| r.e2e);
    let lat = vec![
        tile_line(opt_secs(e2e.map(|e| e.p50)), "p50"),
        tile_line(opt_secs(e2e.map(|e| e.avg)), "avg"),
        tile_line(opt_secs(e2e.map(|e| e.p95)), "p95"),
    ];
    frame.render_widget(Paragraph::new(lat).block(panel("latency · e2e")), cols[1]);

    let ttft = r.and_then(|r| r.ttft);
    let ft = vec![
        tile_line(opt_secs(ttft.map(|t| t.avg)), "avg"),
        tile_line(opt_secs(ttft.map(|t| t.p95)), "p95"),
        tile_line(opt_secs(ttft.map(|t| t.min)), "min"),
    ];
    frame.render_widget(Paragraph::new(ft).block(panel("first token")), cols[2]);
}

fn tile_line(value: String, label: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{value:>9}  "),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::styled(label.to_string(), Style::default().fg(Color::DarkGray)),
    ])
}

fn draw_sparklines(frame: &mut ratatui::Frame, area: Rect, state: &TuiState) {
    let cols =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(area);
    let tps: Vec<u64> = state.tps_hist.iter().copied().collect();
    let ttft: Vec<u64> = state.ttft_hist.iter().copied().collect();
    frame.render_widget(
        Sparkline::default()
            .block(panel("TPS"))
            .data(&tps)
            .style(Style::default().fg(ACCENT)),
        cols[0],
    );
    frame.render_widget(
        Sparkline::default()
            .block(panel("TTFT · ms"))
            .data(&ttft)
            .style(Style::default().fg(Color::Magenta)),
        cols[1],
    );
}

fn draw_distribution(frame: &mut ratatui::Frame, area: Rect, state: &TuiState) {
    let samples: Vec<f64> = state
        .outcomes
        .iter()
        .filter(|o| o.success)
        .map(|o| o.e2e.as_secs_f64())
        .collect();
    let buckets = histogram(&samples, 6);
    let labels: Vec<String> = buckets.iter().map(|(lab, _)| lab.clone()).collect();
    let data: Vec<(&str, u64)> = buckets
        .iter()
        .enumerate()
        .map(|(i, (_, count))| (labels[i].as_str(), *count))
        .collect();
    let chart = BarChart::default()
        .block(panel("e2e distribution"))
        .data(data.as_slice())
        .bar_width(8)
        .bar_gap(1)
        .bar_style(Style::default().fg(ACCENT))
        .value_style(Style::default().add_modifier(Modifier::BOLD));
    frame.render_widget(chart, area);
}

fn draw_requests(frame: &mut ratatui::Frame, area: Rect, state: &TuiState) {
    let header = Row::new(["#", "status", "e2e", "ttft", "tok/s", "tokens", "itl"])
        .style(Style::default().fg(Color::DarkGray));

    let rows = state.sorted_rows();
    let table_rows: Vec<Row> = rows.iter().take(MAX_ROWS).map(row_cells).collect();
    // Selection drives an auto-scrolling TableState: the cursor stays visible
    // however far down the list it moves. Frozen only affects the title.
    let selected = state.selected.min(table_rows.len().saturating_sub(1));
    let mut ts = TableState::default();
    if !table_rows.is_empty() {
        ts.select(Some(selected));
    }

    let title = if state.paused {
        "requests (paused) — ↑/↓ select · enter inspect"
    } else {
        "requests — ↑/↓ select · enter inspect · space pause"
    };
    let widths = [
        Constraint::Length(5),
        Constraint::Length(9),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(8),
        Constraint::Length(9),
    ];
    let table = Table::new(table_rows, widths)
        .header(header)
        .block(panel(title))
        .row_highlight_style(
            Style::default()
                .bg(Color::Indexed(238))
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("› ");
    frame.render_stateful_widget(table, area, &mut ts);
}

fn row_cells(row: &&RowEntry) -> Row<'static> {
    let dash = "—".to_string();
    match &row.outcome {
        None => Row::new(vec![
            Cell::from(row.id.to_string()),
            Cell::from(Span::styled("● live", Style::default().fg(Color::DarkGray))),
            Cell::from(dash.clone()),
            Cell::from(opt_secs(row.ttft.map(|t| t.as_secs_f64()))),
            Cell::from(dash.clone()),
            Cell::from(dash.clone()),
            Cell::from(dash),
        ]),
        Some(o) if o.success => Row::new(vec![
            Cell::from(o.id.to_string()),
            Cell::from(Span::styled("✓ ok", Style::default().fg(Color::Green))),
            Cell::from(format!("{:.2}s", o.e2e.as_secs_f64())),
            Cell::from(opt_secs(o.ttft.map(|t| t.as_secs_f64()))),
            Cell::from(o.tps.map_or(dash.clone(), |v| format!("{v:.1}"))),
            Cell::from(o.completion_tokens.map_or(dash.clone(), |v| v.to_string())),
            Cell::from(o.itl_ms.map_or(dash, |v| format!("{v:.1}ms"))),
        ]),
        Some(o) => {
            let label = o.error.map_or("✗ err".to_string(), |e| format!("✗ {e}"));
            Row::new(vec![
                Cell::from(o.id.to_string()),
                Cell::from(Span::styled(label, Style::default().fg(Color::Red))),
                Cell::from(format!("{:.2}s", o.e2e.as_secs_f64())),
                Cell::from(dash.clone()),
                Cell::from(dash.clone()),
                Cell::from(dash.clone()),
                Cell::from(dash),
            ])
        }
    }
}

fn draw_footer(frame: &mut ratatui::Frame, area: Rect, state: &TuiState) {
    if state.paused {
        let line = Line::from(vec![
            Span::styled(
                " PAUSED ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "  space/p resume · ↑/↓ select · enter inspect · q quit",
                Style::default().fg(Color::DarkGray),
            ),
        ]);
        frame.render_widget(Paragraph::new(line), area);
        return;
    }
    let text = if state.done {
        "DONE — ↑/↓ select · enter inspect · q quit · ? help"
    } else {
        "q quit · space pause · ↑/↓ select · enter inspect · ? help"
    };
    frame.render_widget(
        Paragraph::new(text).style(Style::default().fg(Color::DarkGray)),
        area,
    );
}

/// Full-exchange inspector for the selected request: the JSON payload sent and
/// the model's reply (or the error body), scrollable for long responses.
fn draw_detail(frame: &mut ratatui::Frame, state: &TuiState, cfg: &RunConfig) {
    let rows = state.sorted_rows();
    let Some(row) = rows.get(state.selected.min(rows.len().saturating_sub(1))) else {
        return;
    };

    let area = centered_fixed(frame.area(), 92, 30);
    frame.render_widget(Clear, area);

    let dim = Style::default().fg(Color::DarkGray);
    let head = Style::default().fg(ACCENT).add_modifier(Modifier::BOLD);
    let mut lines: Vec<Line> = Vec::new();

    // --- request (reconstructed; identical for every request) ---------------
    lines.push(Line::from(Span::styled("REQUEST", head)));
    lines.push(Line::from(vec![
        Span::styled("POST ", dim),
        Span::raw(cfg.endpoint.clone()),
    ]));
    lines.push(Line::from(Span::styled(
        "Content-Type: application/json",
        dim,
    )));
    if cfg.api_key.is_some() {
        lines.push(Line::from(Span::styled(
            "Authorization: Bearer ***redacted***",
            dim,
        )));
    }
    for (k, v) in &cfg.headers {
        lines.push(Line::from(Span::styled(format!("{k}: {v}"), dim)));
    }
    lines.push(Line::from(""));
    for l in request_json(cfg).lines() {
        lines.push(Line::from(l.to_string()));
    }
    lines.push(Line::from(""));

    // --- response / error ---------------------------------------------------
    let (status_span, meta) = match &row.outcome {
        None => (
            Span::styled("● in flight", dim),
            "waiting for response…".to_string(),
        ),
        Some(o) if o.success => (
            Span::styled("✓ ok", Style::default().fg(Color::Green)),
            format!(
                "{} · {} tok · {} tok/s",
                opt_secs(Some(o.e2e.as_secs_f64())),
                o.completion_tokens.map_or("—".into(), |t| t.to_string()),
                o.tps.map_or("—".into(), |v| format!("{v:.1}")),
            ),
        ),
        Some(o) => (
            Span::styled(
                o.error.map_or("✗ error".to_string(), |e| format!("✗ {e}")),
                Style::default().fg(Color::Red),
            ),
            opt_secs(Some(o.e2e.as_secs_f64())),
        ),
    };
    lines.push(Line::from(vec![
        Span::styled("RESPONSE  ", head),
        status_span,
        Span::styled(format!("   {meta}"), dim),
    ]));
    lines.push(Line::from(""));
    match row.outcome.as_ref().and_then(|o| o.body.as_deref()) {
        Some(body) => {
            for l in body.lines() {
                lines.push(Line::from(l.to_string()));
            }
        }
        None if row.outcome.is_none() => {}
        None => lines.push(Line::from(Span::styled("(no body)", dim))),
    }

    let title = format!("request #{}  ·  ↑/↓ scroll · esc close", row.id);
    let para = Paragraph::new(lines)
        .block(panel(&title).padding(Padding::new(1, 1, 0, 0)))
        .wrap(Wrap { trim: false })
        .scroll((state.detail_scroll, 0));
    frame.render_widget(para, area);
}

/// Pretty-print the JSON body llmprobe sends (the same for every request).
fn request_json(cfg: &RunConfig) -> String {
    let mut body = serde_json::json!({
        "model": cfg.model,
        "messages": [{ "role": "user", "content": cfg.prompt }],
        "max_tokens": cfg.max_tokens,
        "stream": cfg.stream,
    });
    if let Some(t) = cfg.temperature {
        body["temperature"] = serde_json::json!(t);
    }
    if cfg.stream {
        body["stream_options"] = serde_json::json!({ "include_usage": true });
    }
    serde_json::to_string_pretty(&body).unwrap_or_default()
}

fn draw_help(frame: &mut ratatui::Frame) {
    let area = centered_fixed(frame.area(), 62, 22);
    frame.render_widget(Clear, area);

    // Aligned two-column rows: a styled left key/term, a plain description.
    let key = |k: &str, d: &str| {
        Line::from(vec![
            Span::styled(
                format!("{k:<11}"),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::raw(d.to_string()),
        ])
    };
    let term = |t: &str, d: &str| {
        Line::from(vec![
            Span::styled(
                format!("{t:<11}"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(d.to_string()),
        ])
    };
    let section = |s: &str| {
        Line::from(Span::styled(
            s.to_string(),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ))
    };

    let lines = vec![
        section("KEYS"),
        key("↑ ↓ / j k", "select a request   ·   g top · G bottom"),
        key("enter", "inspect the selected request (full exchange)"),
        key("space / p", "pause sending + freeze the view (read steady)"),
        key("?", "toggle this help"),
        key("q / Esc", "quit — restore terminal, print report"),
        Line::from(""),
        section("METRICS"),
        term("e2e", "full round trip: send → last byte back"),
        term("TTFT", "wait for the first token (streaming)"),
        term("TPS", "tokens/s: per-req decode, or aggregate"),
        term("ITL", "mean gap between tokens (ms); lower = smoother"),
        term("p50 / p95", "half / 95% of requests faster than this"),
        term("speedup", "aggregate ÷ per-req TPS (≈1 = no scaling)"),
        Line::from(""),
        Line::from(Span::styled(
            "press ? or Esc to close",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    frame.render_widget(
        Paragraph::new(lines).block(
            panel("help")
                .title_alignment(Alignment::Center)
                .padding(Padding::new(2, 2, 1, 0)),
        ),
        area,
    );
}

// ---- small helpers --------------------------------------------------------

/// A centered rect of the given size, clamped to fit inside `area`.
fn centered_fixed(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

fn gauge_color(ratio: f64) -> Color {
    if ratio < 0.5 {
        Color::Red
    } else if ratio < 0.85 {
        Color::Yellow
    } else {
        Color::Green
    }
}

fn opt(v: Option<f64>, unit: &str) -> String {
    v.map_or_else(|| "—".to_string(), |x| format!("{x:.1} {unit}"))
}

fn opt_secs(v: Option<f64>) -> String {
    v.map_or_else(|| "—".to_string(), |x| format!("{x:.2}s"))
}

/// Build up to `bins` evenly-spaced e2e buckets with counts and `lo–hi` labels.
fn histogram(samples: &[f64], bins: usize) -> Vec<(String, u64)> {
    if samples.is_empty() || bins == 0 {
        return vec![("—".to_string(), 0)];
    }
    let min = samples.iter().copied().fold(f64::INFINITY, f64::min);
    let max = samples.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if (max - min).abs() < f64::EPSILON {
        return vec![(format!("{min:.2}s"), samples.len() as u64)];
    }
    let width = (max - min) / bins as f64;
    let mut counts = vec![0u64; bins];
    for &s in samples {
        let mut idx = ((s - min) / width) as usize;
        if idx >= bins {
            idx = bins - 1;
        }
        counts[idx] += 1;
    }
    counts
        .into_iter()
        .enumerate()
        .map(|(i, c)| {
            let lo = min + width * i as f64;
            (format!("{lo:.2}"), c)
        })
        .collect()
}
