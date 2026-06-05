//! Live grow-mode dashboard (`#[cfg(feature = "tui")]`).
//!
//! A compact, read-only view: a header, four metric tiles, one aggregate-TPS
//! sparkline, and a scrolling list of conversations. Pause with space/p, quit
//! with q. The authoritative `GrowResult` is returned when the run completes.

use crate::config::RunConfig;
use crate::error::ProbeError;
use crate::fmt::thousands;
use crate::metrics::{
    ConfigSnapshot, ConversationOutcome, GrowResult, RunReport, TerminalReason, TurnOutcome,
    aggregate, mean,
};
use crate::runner::{RunEvent, run_grow};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Cell, Paragraph, Row, Sparkline, Table};
use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::unbounded_channel;
use tokio::sync::watch;

const ACCENT: Color = Color::Cyan;
/// Redraw cadence — kept fast so keypresses echo immediately.
const TICK: Duration = Duration::from_millis(80);
/// Aggregate-recompute cadence. The full report is O(all turns), so it runs at
/// a lower rate than the redraw rather than on every frame.
const REPORT_TICK: Duration = Duration::from_millis(240);
const SPARK_LEN: usize = 60;
/// Most conversation rows shown at once (newest first).
const MAX_CONV_ROWS: usize = 200;

// ── Terminal guard ────────────────────────────────────────────────────────────

struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> Result<Self, ProbeError> {
        // Restore the terminal on panic before the default hook prints, so a
        // crash never leaves the user in raw mode on the alternate screen.
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), LeaveAlternateScreen);
            prev(info);
        }));
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

// ── State ─────────────────────────────────────────────────────────────────────

struct PartialConv {
    slot: usize,
    turns: Vec<TurnOutcome>,
}

/// One display row summarising a conversation's turns.
struct ConvRow {
    conv_id: usize,
    slot: usize,
    turn_count: usize,
    active: bool,
    avg_ttft_ms: Option<f64>,
    avg_tps: Option<f64>,
    completion_tokens: u64,
    terminal: Option<TerminalReason>,
}

fn build_row(
    conv_id: usize,
    slot: usize,
    turns: &[TurnOutcome],
    active: bool,
    terminal: Option<TerminalReason>,
) -> ConvRow {
    let ok = || turns.iter().filter(|t| t.success);
    let ttft: Vec<f64> = ok()
        .filter_map(|t| t.ttft.map(|d| d.as_secs_f64() * 1000.0))
        .collect();
    let tps: Vec<f64> = ok().filter_map(|t| t.tps).collect();
    ConvRow {
        conv_id,
        slot,
        turn_count: turns.len(),
        active,
        avg_ttft_ms: mean(&ttft),
        avg_tps: mean(&tps),
        completion_tokens: ok()
            .filter_map(|t| t.completion_tokens)
            .map(u64::from)
            .sum(),
        terminal,
    }
}

struct TuiState {
    start: Instant,
    cfg_snap: ConfigSnapshot,
    total_conversations: usize,
    partial_convs: BTreeMap<usize, PartialConv>,
    completed_convs: Vec<ConversationOutcome>,
    report: Option<RunReport>,
    last_report: Instant,
    tps_hist: VecDeque<u64>,
    paused: bool,
    paused_total: Duration,
    pause_start: Option<Instant>,
    done: bool,
    final_elapsed: Option<Duration>,
}

impl TuiState {
    fn new(cfg: &RunConfig) -> Self {
        Self {
            start: Instant::now(),
            cfg_snap: ConfigSnapshot {
                endpoint: cfg.endpoint.clone(),
                model: cfg.model.clone(),
                concurrency: cfg.concurrency,
                stream: cfg.stream,
                max_tokens: cfg.max_tokens,
            },
            total_conversations: cfg.conversations,
            partial_convs: BTreeMap::new(),
            completed_convs: Vec::new(),
            report: None,
            last_report: Instant::now(),
            tps_hist: VecDeque::with_capacity(SPARK_LEN),
            paused: false,
            paused_total: Duration::ZERO,
            pause_start: None,
            done: false,
            final_elapsed: None,
        }
    }

    fn infinite(&self) -> bool {
        self.total_conversations == 0
    }

    /// Wall-clock used for rate metrics; excludes paused time, frozen once done.
    fn elapsed(&self) -> Duration {
        if let Some(done) = self.final_elapsed {
            return done;
        }
        let paused = self.paused_total + self.pause_start.map(|t| t.elapsed()).unwrap_or_default();
        self.start.elapsed().saturating_sub(paused)
    }

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
            RunEvent::ConvStarted { conv_id, slot, .. } => {
                self.partial_convs.insert(
                    conv_id,
                    PartialConv {
                        slot,
                        turns: Vec::new(),
                    },
                );
            }
            RunEvent::TurnStarted { .. } | RunEvent::TurnFirstToken { .. } => {}
            RunEvent::TurnFinished {
                conv_id, outcome, ..
            } => {
                if let Some(p) = self.partial_convs.get_mut(&conv_id) {
                    p.turns.push(outcome);
                }
            }
            RunEvent::ConvFinished { conv_id, outcome } => {
                self.partial_convs.remove(&conv_id);
                self.completed_convs.push(outcome);
                if !self.infinite()
                    && self.completed_convs.len() >= self.total_conversations
                    && !self.done
                {
                    self.done = true;
                    self.final_elapsed = Some(self.elapsed());
                }
            }
        }
    }

    /// Partial `GrowResult` for live metric computation.
    fn snapshot_result(&self) -> GrowResult {
        let mut conversations = self.completed_convs.clone();
        for (id, p) in &self.partial_convs {
            if !p.turns.is_empty() {
                conversations.push(ConversationOutcome {
                    id: *id,
                    slot: p.slot,
                    system: String::new(),
                    turns: p.turns.clone(),
                    terminal: TerminalReason::Cancelled,
                    wall_clock: Duration::ZERO,
                });
            }
        }
        GrowResult {
            conversations,
            wall_clock: self.elapsed(),
            config: self.cfg_snap.clone(),
        }
    }

    /// Recompute metrics and push a sparkline sample. No-op while paused.
    fn on_tick(&mut self) {
        if self.paused {
            return;
        }
        if self.report.is_some() && self.last_report.elapsed() < REPORT_TICK {
            return;
        }
        self.last_report = Instant::now();
        self.report = Some(aggregate(&self.snapshot_result()));
        if !self.done
            && let Some(ref r) = self.report
        {
            let tps = r.turns.aggregate_tps.unwrap_or(0.0).round() as u64;
            if self.tps_hist.len() == SPARK_LEN {
                self.tps_hist.pop_front();
            }
            self.tps_hist.push_back(tps);
        }
    }

    /// Conversation rows, newest first, capped for display.
    fn conv_rows(&self) -> Vec<ConvRow> {
        let mut rows: Vec<ConvRow> = Vec::new();
        for p in &self.partial_convs {
            rows.push(build_row(*p.0, p.1.slot, &p.1.turns, true, None));
        }
        for conv in self.completed_convs.iter().rev() {
            rows.push(build_row(
                conv.id,
                conv.slot,
                &conv.turns,
                false,
                Some(conv.terminal.clone()),
            ));
            if rows.len() >= MAX_CONV_ROWS {
                break;
            }
        }
        rows
    }
}

// ── Public entry point ──────────────────────────────────────────────────────

/// Run the live grow-mode TUI and return the authoritative `GrowResult`.
pub async fn run(cfg: &RunConfig) -> Result<GrowResult, ProbeError> {
    let _guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend).map_err(io_err)?;

    let (run_tx, mut run_rx) = unbounded_channel::<RunEvent>();
    let (pause_tx, pause_rx) = watch::channel(false);
    let cfg_task = cfg.clone();
    let mut grow_task =
        tokio::spawn(async move { run_grow(&cfg_task, Some(run_tx), Some(pause_rx)).await });

    let (key_tx, mut key_rx) = unbounded_channel::<Event>();
    std::thread::spawn(move || {
        while let Ok(ev) = event::read() {
            if key_tx.send(ev).is_err() {
                break;
            }
        }
    });

    let mut state = TuiState::new(cfg);
    let mut ticker = tokio::time::interval(TICK);
    let mut grow_result: Option<Result<GrowResult, ProbeError>> = None;

    loop {
        tokio::select! {
            res = &mut grow_task, if grow_result.is_none() => {
                grow_result = Some(match res {
                    Ok(inner) => inner,
                    Err(e) => Err(ProbeError::Stream(format!("runner task failed: {e}"))),
                });
            }
            Some(ev) = run_rx.recv() => state.apply(ev),
            Some(ev) = key_rx.recv() => {
                if let Event::Key(k) = ev
                    && k.kind == KeyEventKind::Press
                {
                    let ctrl_c = k.code == KeyCode::Char('c')
                        && k.modifiers.contains(KeyModifiers::CONTROL);
                    if ctrl_c || handle_key(&mut state, k.code, &pause_tx) {
                        break;
                    }
                }
            }
            _ = ticker.tick() => {
                state.on_tick();
                terminal.draw(|f| draw(f, &state)).map_err(io_err)?;
            }
        }
    }

    match grow_result {
        Some(Ok(result)) => Ok(result),
        Some(Err(e)) => Err(e),
        None => {
            grow_task.abort();
            Ok(state.snapshot_result())
        }
    }
}

/// Returns `true` when the key means "quit".
fn handle_key(state: &mut TuiState, code: KeyCode, pause_tx: &watch::Sender<bool>) -> bool {
    match code {
        KeyCode::Char('q') | KeyCode::Esc => return true,
        KeyCode::Char('p') | KeyCode::Char(' ') => {
            state.toggle_pause();
            let _ = pause_tx.send(state.paused);
        }
        _ => {}
    }
    false
}

// ── Rendering ─────────────────────────────────────────────────────────────────

fn draw(frame: &mut ratatui::Frame, state: &TuiState) {
    let areas = Layout::vertical([
        Constraint::Length(3), // header
        Constraint::Length(4), // metric tiles
        Constraint::Length(4), // sparkline
        Constraint::Min(5),    // conversation list
        Constraint::Length(1), // footer
    ])
    .split(frame.area());

    draw_header(frame, areas[0], state);
    draw_tiles(frame, areas[1], state);
    draw_sparkline(frame, areas[2], state);
    draw_conversations(frame, areas[3], state);
    draw_footer(frame, areas[4], state);
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

fn draw_header(frame: &mut ratatui::Frame, area: Rect, state: &TuiState) {
    let elapsed = state.elapsed().as_secs_f64();
    let (mode_str, mode_bg) = if state.paused {
        (" PAUSED ", Color::Yellow)
    } else if state.done {
        ("  DONE  ", Color::Green)
    } else {
        ("  LIVE  ", Color::Green)
    };

    let line = Line::from(vec![
        Span::raw(format!(
            "  {} · {} · c={} · stream={}",
            state.cfg_snap.endpoint,
            state.cfg_snap.model,
            state.cfg_snap.concurrency,
            state.cfg_snap.stream,
        )),
        Span::raw("    "),
        Span::styled(
            format!("⏱ {elapsed:.1}s"),
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::raw("    "),
        Span::styled(
            mode_str,
            Style::default()
                .fg(Color::Black)
                .bg(mode_bg)
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    frame.render_widget(Paragraph::new(line).block(panel("llmprobe")), area);
}

fn draw_tiles(frame: &mut ratatui::Frame, area: Rect, state: &TuiState) {
    let cols = Layout::horizontal([
        Constraint::Ratio(1, 4),
        Constraint::Ratio(1, 4),
        Constraint::Ratio(1, 4),
        Constraint::Ratio(1, 4),
    ])
    .split(area);

    let r = state.report.as_ref();

    let ttft = r.and_then(|r| r.turns.ttft);
    frame.render_widget(
        Paragraph::new(vec![
            tile_line(ms_val(ttft.map(|t| t.p50)), "p50"),
            tile_line(ms_val(ttft.map(|t| t.p95)), "p95"),
            tile_line(ms_val(ttft.map(|t| t.p99)), "p99"),
        ])
        .block(panel("TTFT")),
        cols[0],
    );

    let tps = r.and_then(|r| r.turns.tps);
    let agg_tps = r.and_then(|r| r.turns.aggregate_tps);
    frame.render_widget(
        Paragraph::new(vec![
            tile_line(tps_val(tps.map(|t| t.p50)), "p50 per-req"),
            tile_line(tps_val(tps.map(|t| t.p95)), "p95 per-req"),
            tile_line(tps_val(agg_tps), "aggregate"),
        ])
        .block(panel("TPS tok/s")),
        cols[1],
    );

    let ok_turns = r.map(|r| r.turns.ok_turns).unwrap_or(0);
    let total_turns = r.map(|r| r.turns.total_turns).unwrap_or(0);
    let success_pct = if total_turns > 0 {
        ok_turns as f64 / total_turns as f64 * 100.0
    } else {
        100.0
    };
    let active = state.partial_convs.len();
    let concurrency = state.cfg_snap.concurrency;
    let completed = state.completed_convs.len();
    frame.render_widget(
        Paragraph::new(vec![
            tile_line(format!("{success_pct:.1}%"), "success rate"),
            tile_line(format!("{active}/{concurrency}"), "active slots"),
            tile_line(completed.to_string(), "convs done"),
        ])
        .block(panel("Status")),
        cols[2],
    );

    let total_tok = r.map(|r| r.turns.total_completion_tokens).unwrap_or(0);
    let target = if state.infinite() {
        "∞".to_string()
    } else {
        state.total_conversations.to_string()
    };
    frame.render_widget(
        Paragraph::new(vec![
            tile_line(thousands(total_tok), "compl tokens"),
            tile_line(format!("{ok_turns}/{total_turns}"), "turns ok/total"),
            tile_line(target, "target convs"),
        ])
        .block(panel("Throughput")),
        cols[3],
    );
}

fn tile_line(value: String, label: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("  {value:>10}  "),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::styled(label.to_string(), Style::default().fg(Color::DarkGray)),
    ])
}

fn draw_sparkline(frame: &mut ratatui::Frame, area: Rect, state: &TuiState) {
    let tps: Vec<u64> = state.tps_hist.iter().copied().collect();
    let now = tps.last().copied().unwrap_or(0);
    let peak = tps.iter().copied().max().unwrap_or(0);
    let title = format!("Aggregate TPS · now {now} · peak {peak}");
    frame.render_widget(
        Sparkline::default()
            .block(panel(&title))
            .data(&tps)
            .style(Style::default().fg(ACCENT)),
        area,
    );
}

fn draw_conversations(frame: &mut ratatui::Frame, area: Rect, state: &TuiState) {
    let rows = state.conv_rows();
    let dash = "—";

    let header = Row::new([
        "#",
        "slot",
        "turns",
        "state",
        "TTFT avg",
        "TPS avg",
        "compl-tok",
        "terminal",
    ])
    .style(Style::default().fg(Color::DarkGray))
    .height(1);

    let table_rows: Vec<Row> = rows
        .iter()
        .map(|r| {
            let state_cell = if r.active {
                Cell::from(Span::styled("● active", Style::default().fg(Color::Cyan)))
            } else {
                Cell::from(Span::styled("✓ done", Style::default().fg(Color::Green)))
            };
            let terminal_cell = match &r.terminal {
                Some(TerminalReason::ContextLimit) => {
                    Cell::from(Span::styled("ctx-limit", Style::default().fg(Color::Green)))
                }
                Some(TerminalReason::MaxTurns) => Cell::from(Span::styled(
                    "max-turns",
                    Style::default().fg(Color::Yellow),
                )),
                Some(t) => Cell::from(Span::styled(t.to_string(), Style::default().fg(Color::Red))),
                None => Cell::from(dash),
            };
            let ct = if r.completion_tokens > 0 {
                thousands(r.completion_tokens)
            } else {
                dash.to_string()
            };
            Row::new(vec![
                Cell::from(r.conv_id.to_string()),
                Cell::from(format!("[{}]", r.slot)),
                Cell::from(r.turn_count.to_string()),
                state_cell,
                Cell::from(
                    r.avg_ttft_ms
                        .map_or(dash.to_string(), |v| format!("{v:.0}ms")),
                ),
                Cell::from(r.avg_tps.map_or(dash.to_string(), |v| format!("{v:.1}"))),
                Cell::from(ct),
                terminal_cell,
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(6),
        Constraint::Length(5),
        Constraint::Length(6),
        Constraint::Length(9),
        Constraint::Length(9),
        Constraint::Length(8),
        Constraint::Length(10),
        Constraint::Min(9),
    ];
    let table = Table::new(table_rows, widths)
        .header(header)
        .block(panel("conversations (newest first)"));
    frame.render_widget(table, area);
}

fn draw_footer(frame: &mut ratatui::Frame, area: Rect, state: &TuiState) {
    let text = if state.paused {
        " PAUSED — space/p resume  ·  q quit"
    } else if state.done {
        " DONE — q quit (report printed after)"
    } else {
        " q quit  ·  space/p pause"
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            text,
            Style::default().fg(Color::DarkGray),
        ))),
        area,
    );
}

fn ms_val(v: Option<f64>) -> String {
    v.map_or_else(|| "—".to_string(), |x| format!("{x:.0}ms"))
}

fn tps_val(v: Option<f64>) -> String {
    v.map_or_else(|| "—".to_string(), |x| format!("{x:.1}"))
}
