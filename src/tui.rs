//! Live grow-mode dashboard (`--tui`, `#[cfg(feature = "tui")]`).

use crate::config::RunConfig;
use crate::error::ProbeError;
use crate::fmt::thousands;
use crate::metrics::{
    aggregate, mean, request_messages, ConfigSnapshot, ConversationOutcome, GrowResult, RunReport,
    TerminalReason, TurnOutcome,
};
use crate::runner::{RunEvent, run_grow};
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
    Block, BorderType, Borders, Cell, Clear, Padding, Paragraph, Row, Sparkline, Table, TableState,
    Wrap,
};
use std::collections::{BTreeMap, VecDeque};
use std::io;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::unbounded_channel;
use tokio::sync::watch;

const ACCENT: Color = Color::Cyan;
/// Redraw cadence — kept fast so keypresses echo immediately.
const TICK: Duration = Duration::from_millis(80);
/// Aggregate-recompute cadence. The full report is O(all turns), so it runs
/// at a lower rate than the redraw rather than on every frame.
const REPORT_TICK: Duration = Duration::from_millis(240);
const SPARK_LEN: usize = 60;
const MAX_CONV_ROWS: usize = 500;
/// Fixed size of the conversation-detail modal.
const DETAIL_W: u16 = 102;
const DETAIL_H: u16 = 34;
/// Fixed size of the per-turn request/response modal (roomier for long replies).
const TURN_W: u16 = 110;
const TURN_H: u16 = 40;

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

// ── State types ───────────────────────────────────────────────────────────────

struct PartialConv {
    conv_id: usize,
    slot: usize,
    turns: Vec<TurnOutcome>,
}

struct ConvRow<'a> {
    conv_id: usize,
    slot: usize,
    turn_count: usize,
    active: bool,
    avg_ttft_ms: Option<f64>,
    avg_tps: Option<f64>,
    total_prompt_tokens: u64,
    total_completion_tokens: u64,
    terminal: Option<TerminalReason>,
    /// Borrowed from the owning conversation — never cloned per frame.
    turns: &'a [TurnOutcome],
}

/// Build a display row by summarising a conversation's turns. Borrows `turns`.
fn build_row(
    conv_id: usize,
    slot: usize,
    turns: &[TurnOutcome],
    active: bool,
    terminal: Option<TerminalReason>,
) -> ConvRow<'_> {
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
        total_prompt_tokens: ok().filter_map(|t| t.prompt_tokens).map(u64::from).sum(),
        total_completion_tokens: ok().filter_map(|t| t.completion_tokens).map(u64::from).sum(),
        terminal,
        turns,
    }
}

/// Mutually exclusive display phase, derived once from the underlying flags so
/// the precedence (replay > paused > done > live) lives in a single place.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
    Live,
    Paused,
    Done,
    Replay,
}

/// Sort order for the conversation table, cycled with `s`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SortKey {
    /// Most-recent conversation first (default).
    Recent,
    /// Most turns first.
    Turns,
    /// Slowest average TTFT first.
    Ttft,
    /// Slowest (lowest) average TPS first.
    Tps,
}

impl SortKey {
    fn next(self) -> Self {
        match self {
            SortKey::Recent => SortKey::Turns,
            SortKey::Turns => SortKey::Ttft,
            SortKey::Ttft => SortKey::Tps,
            SortKey::Tps => SortKey::Recent,
        }
    }

    fn label(self) -> &'static str {
        match self {
            SortKey::Recent => "recent",
            SortKey::Turns => "turns",
            SortKey::Ttft => "TTFT",
            SortKey::Tps => "TPS",
        }
    }
}

struct TuiState {
    start: Instant,
    cfg_snap: ConfigSnapshot,
    total_conversations: usize,
    partial_convs: BTreeMap<usize, PartialConv>,
    completed_convs: Vec<ConversationOutcome>,
    report: Option<RunReport>,
    /// When the aggregate report was last recomputed (throttled to REPORT_TICK).
    last_report: Instant,
    tps_hist: VecDeque<u64>,
    ttft_hist: VecDeque<u64>,
    selected: usize,
    sort: SortKey,
    show_detail: bool,
    /// Selected turn row inside the detail modal.
    detail_turn: usize,
    /// Request/response drill-down for the selected turn is open.
    show_turn: bool,
    /// Scroll offset inside the request/response view.
    turn_scroll: u16,
    /// false = last user message only, true = full request payload.
    turn_expanded: bool,
    show_help: bool,
    paused: bool,
    /// Accumulated pause duration across all pause/resume cycles.
    paused_total: Duration,
    /// Wall-clock timestamp of the current pause start, if paused.
    pause_start: Option<Instant>,
    done: bool,
    /// Frozen elapsed time captured the moment the run completes.
    final_elapsed: Option<Duration>,
    is_replay: bool,
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
            ttft_hist: VecDeque::with_capacity(SPARK_LEN),
            selected: 0,
            sort: SortKey::Recent,
            show_detail: false,
            detail_turn: 0,
            show_turn: false,
            turn_scroll: 0,
            turn_expanded: false,
            show_help: false,
            paused: false,
            paused_total: Duration::ZERO,
            pause_start: None,
            done: false,
            final_elapsed: None,
            is_replay: false,
        }
    }

    fn from_result(result: &GrowResult) -> Self {
        let mut s = Self {
            start: Instant::now(),
            cfg_snap: result.config.clone(),
            total_conversations: result.conversations.len(),
            partial_convs: BTreeMap::new(),
            completed_convs: result.conversations.clone(),
            report: None,
            last_report: Instant::now(),
            tps_hist: VecDeque::with_capacity(SPARK_LEN),
            ttft_hist: VecDeque::with_capacity(SPARK_LEN),
            selected: 0,
            sort: SortKey::Recent,
            show_detail: false,
            detail_turn: 0,
            show_turn: false,
            turn_scroll: 0,
            turn_expanded: false,
            show_help: false,
            paused: false,
            paused_total: Duration::ZERO,
            pause_start: None,
            done: true,
            final_elapsed: Some(result.wall_clock),
            is_replay: true,
        };
        s.recompute_report();
        s
    }

    fn infinite(&self) -> bool {
        self.total_conversations == 0
    }

    /// Current display phase. Precedence is defined here and nowhere else.
    fn phase(&self) -> Phase {
        if self.is_replay {
            Phase::Replay
        } else if self.paused {
            Phase::Paused
        } else if self.done {
            Phase::Done
        } else {
            Phase::Live
        }
    }

    /// Wall-clock used for rate metrics.
    /// Excludes time spent paused; frozen once the run completes.
    fn elapsed(&self) -> Duration {
        if let Some(done) = self.final_elapsed {
            return done;
        }
        let paused = self.paused_total
            + self.pause_start.map(|t| t.elapsed()).unwrap_or_default();
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
            RunEvent::ConvStarted { conv_id, slot } => {
                self.partial_convs.insert(
                    conv_id,
                    PartialConv { conv_id, slot, turns: Vec::new() },
                );
            }
            RunEvent::TurnStarted { .. } | RunEvent::TurnFirstToken { .. } => {}
            RunEvent::TurnFinished { conv_id, outcome, .. } => {
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

    fn recompute_report(&mut self) {
        let snap = self.snapshot_result();
        self.report = Some(aggregate(&snap));
    }

    /// Partial `GrowResult` for live metric computation.
    /// Uses `self.elapsed()` as wall_clock so aggregate TPS is pause-aware.
    fn snapshot_result(&self) -> GrowResult {
        let mut conversations = self.completed_convs.clone();
        for p in self.partial_convs.values() {
            if !p.turns.is_empty() {
                conversations.push(ConversationOutcome {
                    id: p.conv_id,
                    slot: p.slot,
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

    /// Called every tick: recompute metrics and push sparkline samples.
    /// No-op while paused so the display freezes cleanly.
    fn on_tick(&mut self) {
        if self.paused {
            return;
        }
        // Throttle the O(all turns) aggregate to REPORT_TICK; the redraw itself
        // keeps running at the faster TICK rate for input responsiveness.
        if self.report.is_some() && self.last_report.elapsed() < REPORT_TICK {
            return;
        }
        self.last_report = Instant::now();
        self.recompute_report();
        if !self.done {
            if let Some(ref r) = self.report {
                push_capped(
                    &mut self.tps_hist,
                    r.turns.aggregate_tps.unwrap_or(0.0).round() as u64,
                );
                let ttft_ms = r.turns.ttft.map(|t| t.avg.round() as u64).unwrap_or(0);
                push_capped(&mut self.ttft_hist, ttft_ms);
            }
        }
    }

    fn conv_rows(&self) -> Vec<ConvRow<'_>> {
        let mut rows: Vec<ConvRow> = Vec::new();
        for conv in &self.completed_convs {
            rows.push(build_row(
                conv.id,
                conv.slot,
                &conv.turns,
                false,
                Some(conv.terminal.clone()),
            ));
        }
        for p in self.partial_convs.values() {
            rows.push(build_row(p.conv_id, p.slot, &p.turns, true, None));
        }
        // conv_id is the stable tie-breaker so equal rows never jitter, and
        // rows missing the sort metric (`None`) always sink to the bottom.
        match self.sort {
            SortKey::Recent => rows.sort_by(|a, b| b.conv_id.cmp(&a.conv_id)),
            SortKey::Turns => rows.sort_by(|a, b| {
                b.turn_count
                    .cmp(&a.turn_count)
                    .then(b.conv_id.cmp(&a.conv_id))
            }),
            SortKey::Ttft => rows.sort_by(|a, b| {
                // Slowest TTFT first: descending, None ranked lowest.
                let av = a.avg_ttft_ms.unwrap_or(f64::NEG_INFINITY);
                let bv = b.avg_ttft_ms.unwrap_or(f64::NEG_INFINITY);
                bv.total_cmp(&av).then(b.conv_id.cmp(&a.conv_id))
            }),
            SortKey::Tps => rows.sort_by(|a, b| {
                // Slowest TPS first: ascending, None ranked highest (last).
                let av = a.avg_tps.unwrap_or(f64::INFINITY);
                let bv = b.avg_tps.unwrap_or(f64::INFINITY);
                av.total_cmp(&bv).then(b.conv_id.cmp(&a.conv_id))
            }),
        }
        rows.truncate(MAX_CONV_ROWS);
        rows
    }

    /// Number of turns in the currently selected conversation (clamps the
    /// turn cursor inside the detail modal).
    fn detail_turn_count(&self) -> usize {
        let rows = self.conv_rows();
        let selected = self.selected.min(rows.len().saturating_sub(1));
        rows.get(selected).map_or(0, |r| r.turns.len())
    }

    /// Styled body of the request/response view for the selected turn. Shared by
    /// `draw_turn` (rendering) and `turn_max_scroll` (line counting) so the two
    /// can never drift apart.
    fn turn_lines(&self) -> Vec<Line<'static>> {
        let rows = self.conv_rows();
        let selected = self.selected.min(rows.len().saturating_sub(1));
        let Some(row) = rows.get(selected) else {
            return Vec::new();
        };
        let turn_idx = self.detail_turn.min(row.turns.len().saturating_sub(1));
        let Some(turn) = row.turns.get(turn_idx) else {
            return Vec::new();
        };

        let dim = Style::default().fg(Color::DarkGray);
        let accent = Style::default().fg(ACCENT);
        // Pads a label out to the modal's inner width with a box-drawing rule.
        let divider = |label: String| -> Line<'static> {
            const INNER: usize = TURN_W as usize - 3;
            let fill = INNER.saturating_sub(label.chars().count());
            Line::from(Span::styled(format!("{label}{}", "─".repeat(fill)), dim))
        };
        let body = |text: &str| -> Vec<Line<'static>> {
            text.lines()
                .map(|l| Line::from(Span::raw(format!("  {l}"))))
                .collect()
        };

        let mut lines: Vec<Line<'static>> = Vec::new();

        // Header: status + this turn's metrics.
        let status = if turn.success {
            Span::styled("✓ ok", Style::default().fg(Color::Green))
        } else {
            let label = turn.error.map_or("✗ error".to_string(), |e| format!("✗ {e}"));
            Span::styled(label, Style::default().fg(Color::Red))
        };
        let mut meta = String::new();
        if let Some(v) = turn.prompt_tokens {
            meta.push_str(&format!("  prompt {} tok", thousands(u64::from(v))));
        }
        if let Some(v) = turn.completion_tokens {
            meta.push_str(&format!(" · compl {} tok", thousands(u64::from(v))));
        }
        meta.push_str(&format!(" · e2e {:.2}s", turn.e2e.as_secs_f64()));
        if let Some(d) = turn.ttft {
            meta.push_str(&format!(" · ttft {:.0}ms", d.as_secs_f64() * 1000.0));
        }
        if let Some(v) = turn.tps {
            meta.push_str(&format!(" · tps {v:.1}"));
        }
        lines.push(Line::from(vec![
            Span::raw("  "),
            status,
            Span::styled(meta, dim),
        ]));
        lines.push(Line::from(""));

        // REQUEST — last user message by default, full payload when expanded.
        let msgs = request_messages(row.turns, turn_idx);
        if self.turn_expanded {
            lines.push(divider(format!(
                "  ── REQUEST ({} messages · x to collapse) ",
                msgs.len()
            )));
            for (role, content) in &msgs {
                lines.push(Line::from(Span::styled(format!("  [{role}]"), accent)));
                lines.extend(body(content));
            }
        } else {
            // Only advertise expansion when the full payload holds more than the
            // single message already on screen.
            let label = if msgs.len() > 1 {
                format!(
                    "  ── REQUEST (last of {} messages · x to expand) ",
                    msgs.len()
                )
            } else {
                "  ── REQUEST (user) ".to_string()
            };
            lines.push(divider(label));
            let last = msgs.last().map(|(_, c)| c.as_str()).unwrap_or("");
            if last.is_empty() {
                lines.push(Line::from(Span::styled("  (empty)", dim)));
            } else {
                lines.extend(body(last));
            }
        }
        lines.push(Line::from(""));

        // RESPONSE — the captured reply, or the error for a failed turn.
        lines.push(divider("  ── RESPONSE (assistant) ".into()));
        if turn.success {
            match turn.reply.as_deref() {
                Some(r) if !r.is_empty() => lines.extend(body(r)),
                _ => lines.push(Line::from(Span::styled("  (empty response)", dim))),
            }
        } else {
            let label = turn
                .error
                .map_or("request failed".to_string(), |e| format!("request failed: {e}"));
            lines.push(Line::from(Span::styled(
                format!("  {label}"),
                Style::default().fg(Color::Red),
            )));
        }

        lines
    }

    /// Largest valid scroll offset for the request/response view, so navigation
    /// can't run past the end of the content. Counts wrapped rows for the modal's
    /// fixed inner width (an upper bound, so the last line stays reachable).
    fn turn_max_scroll(&self) -> u16 {
        const INNER: usize = TURN_W as usize - 3;
        const VIEWPORT: usize = TURN_H as usize - 2;
        let total: usize = self
            .turn_lines()
            .iter()
            .map(|l| l.width().max(1).div_ceil(INNER))
            .sum();
        total.saturating_sub(VIEWPORT) as u16
    }
}

fn push_capped(buf: &mut VecDeque<u64>, v: u64) {
    if buf.len() == SPARK_LEN {
        buf.pop_front();
    }
    buf.push_back(v);
}

// ── Public entry points ───────────────────────────────────────────────────────

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

/// Open an interactive replay of a saved `GrowResult` (no HTTP requests made).
pub async fn replay(result: &GrowResult) -> Result<(), ProbeError> {
    let _guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend).map_err(io_err)?;

    let (key_tx, mut key_rx) = unbounded_channel::<Event>();
    std::thread::spawn(move || {
        while let Ok(ev) = event::read() {
            if key_tx.send(ev).is_err() {
                break;
            }
        }
    });

    let mut state = TuiState::from_result(result);
    let mut ticker = tokio::time::interval(TICK);
    // Replay has no pause — use a dummy sender that is immediately dropped.
    let (no_pause, _) = watch::channel(false);

    loop {
        tokio::select! {
            Some(ev) = key_rx.recv() => {
                if let Event::Key(k) = ev
                    && k.kind == KeyEventKind::Press
                {
                    if handle_key(&mut state, k.code, &no_pause) {
                        break;
                    }
                }
            }
            _ = ticker.tick() => {
                terminal.draw(|f| draw(f, &state)).map_err(io_err)?;
            }
        }
    }

    Ok(())
}

// ── Key handling ──────────────────────────────────────────────────────────────

/// Returns `true` when the key means "quit".
fn handle_key(state: &mut TuiState, code: KeyCode, pause_tx: &watch::Sender<bool>) -> bool {
    if state.show_help {
        if matches!(code, KeyCode::Char('?') | KeyCode::Esc | KeyCode::Char('q')) {
            state.show_help = false;
        }
        return false;
    }
    // Innermost view first: request/response for one turn.
    if state.show_turn {
        match code {
            KeyCode::Esc | KeyCode::Char('q') => state.show_turn = false,
            KeyCode::Up | KeyCode::Char('k') => {
                state.turn_scroll = state.turn_scroll.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                state.turn_scroll = (state.turn_scroll + 1).min(state.turn_max_scroll());
            }
            KeyCode::Char('g') => state.turn_scroll = 0,
            KeyCode::Char('G') => state.turn_scroll = state.turn_max_scroll(),
            KeyCode::Char('x') => {
                state.turn_expanded = !state.turn_expanded;
                state.turn_scroll = 0;
            }
            _ => {}
        }
        return false;
    }
    // Detail modal: ↑/↓ select a turn, Enter drills into it.
    if state.show_detail {
        let last = state.detail_turn_count().saturating_sub(1);
        match code {
            KeyCode::Esc | KeyCode::Char('q') => state.show_detail = false,
            KeyCode::Up | KeyCode::Char('k') => {
                state.detail_turn = state.detail_turn.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                state.detail_turn = (state.detail_turn + 1).min(last);
            }
            KeyCode::Char('g') => state.detail_turn = 0,
            KeyCode::Char('G') => state.detail_turn = last,
            KeyCode::Enter if state.detail_turn_count() > 0 => {
                state.show_turn = true;
                state.turn_scroll = 0;
                state.turn_expanded = false;
            }
            _ => {}
        }
        return false;
    }

    let row_count = state.conv_rows().len();
    let last = row_count.saturating_sub(1);

    match code {
        KeyCode::Char('q') | KeyCode::Esc => return true,
        KeyCode::Char('p') | KeyCode::Char(' ') if !state.is_replay => {
            state.toggle_pause();
            let _ = pause_tx.send(state.paused);
        }
        KeyCode::Char('s') => state.sort = state.sort.next(),
        KeyCode::Char('?') => state.show_help = true,
        KeyCode::Enter if row_count > 0 => {
            state.show_detail = true;
            state.detail_turn = 0;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            state.selected = state.selected.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            state.selected = (state.selected + 1).min(last);
        }
        KeyCode::Char('g') => state.selected = 0,
        KeyCode::Char('G') => state.selected = last,
        _ => {}
    }
    false
}

// ── Rendering ─────────────────────────────────────────────────────────────────

fn draw(frame: &mut ratatui::Frame, state: &TuiState) {
    let areas = Layout::vertical([
        Constraint::Length(3), // header
        Constraint::Length(4), // metric tiles
        Constraint::Length(4), // sparklines
        Constraint::Min(5),    // conversation table
        Constraint::Length(1), // footer
    ])
    .split(frame.area());

    // Built once per frame and shared by the table and the detail modal.
    let rows = state.conv_rows();

    draw_header(frame, areas[0], state);
    draw_tiles(frame, areas[1], state);
    draw_sparklines(frame, areas[2], state);
    draw_conversations(frame, areas[3], state, &rows);
    draw_footer(frame, areas[4], state);

    if state.show_detail {
        draw_detail(frame, state, &rows);
    }
    if state.show_turn {
        draw_turn(frame, state, &rows);
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

fn draw_header(frame: &mut ratatui::Frame, area: Rect, state: &TuiState) {
    let elapsed = state.elapsed().as_secs_f64();
    let (mode_str, mode_fg, mode_bg) = match state.phase() {
        Phase::Replay => (" REPLAY ", ACCENT, Color::Reset),
        Phase::Paused => (" PAUSED ", Color::Black, Color::Yellow),
        Phase::Done => ("  DONE  ", Color::Black, Color::Green),
        Phase::Live => ("  LIVE  ", Color::Black, Color::Green),
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
                .fg(mode_fg)
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

    // TTFT tile
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

    // TPS tile
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

    // Status tile
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

    // Throughput tile
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

fn draw_sparklines(frame: &mut ratatui::Frame, area: Rect, state: &TuiState) {
    let cols =
        Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).split(area);
    let tps: Vec<u64> = state.tps_hist.iter().copied().collect();
    let ttft: Vec<u64> = state.ttft_hist.iter().copied().collect();

    // A bare sparkline shape is unreadable without a scale, so surface the
    // current and peak sample in each panel title.
    let scale = |series: &[u64], unit: &str| {
        let now = series.last().copied().unwrap_or(0);
        let peak = series.iter().copied().max().unwrap_or(0);
        format!("now {now}{unit} · peak {peak}{unit}")
    };

    let tps_title = format!("Agg TPS · {}", scale(&tps, ""));
    frame.render_widget(
        Sparkline::default()
            .block(panel(&tps_title))
            .data(&tps)
            .style(Style::default().fg(ACCENT)),
        cols[0],
    );
    let ttft_title = format!("TTFT avg · {}", scale(&ttft, "ms"));
    frame.render_widget(
        Sparkline::default()
            .block(panel(&ttft_title))
            .data(&ttft)
            .style(Style::default().fg(Color::Magenta)),
        cols[1],
    );
}

fn draw_conversations(frame: &mut ratatui::Frame, area: Rect, state: &TuiState, rows: &[ConvRow]) {
    let selected = state.selected.min(rows.len().saturating_sub(1));

    let header = Row::new([
        "#", "slot", "turns", "state", "TTFT avg", "TPS avg",
        "prompt-tok", "compl-tok", "terminal",
    ])
    .style(Style::default().fg(Color::DarkGray))
    .height(1);

    let table_rows: Vec<Row> = rows
        .iter()
        .map(|r| {
            let dash = "—";
            let state_cell = if r.active {
                Cell::from(Span::styled("● active", Style::default().fg(Color::Cyan)))
            } else {
                Cell::from(Span::styled("✓ done", Style::default().fg(Color::Green)))
            };
            let terminal_cell = match &r.terminal {
                Some(TerminalReason::ContextLimit) => Cell::from(Span::styled(
                    "ctx-limit",
                    Style::default().fg(Color::Green),
                )),
                Some(TerminalReason::MaxTurns) => Cell::from(Span::styled(
                    "max-turns",
                    Style::default().fg(Color::Yellow),
                )),
                Some(t) => Cell::from(Span::styled(
                    t.to_string(),
                    Style::default().fg(Color::Red),
                )),
                None => Cell::from(dash),
            };
            let pt = if r.total_prompt_tokens > 0 {
                thousands(r.total_prompt_tokens)
            } else {
                dash.to_string()
            };
            let ct = if r.total_completion_tokens > 0 {
                thousands(r.total_completion_tokens)
            } else {
                dash.to_string()
            };
            Row::new(vec![
                Cell::from(r.conv_id.to_string()),
                Cell::from(format!("[{}]", r.slot)),
                Cell::from(r.turn_count.to_string()),
                state_cell,
                Cell::from(r.avg_ttft_ms.map_or(dash.to_string(), |v| format!("{v:.0}ms"))),
                Cell::from(r.avg_tps.map_or(dash.to_string(), |v| format!("{v:.1}"))),
                Cell::from(pt),
                Cell::from(ct),
                terminal_cell,
            ])
        })
        .collect();

    let mut ts = TableState::default();
    if !table_rows.is_empty() {
        ts.select(Some(selected));
    }

    let title = match state.phase() {
        Phase::Replay => "conversations [REPLAY] — ↑/↓ select  ·  enter: turn detail  ·  q quit",
        Phase::Paused => {
            "conversations [PAUSED] — ↑/↓ select  ·  enter: turn detail  ·  space: resume"
        }
        Phase::Done | Phase::Live => {
            "conversations — ↑/↓ select  ·  enter: turn detail  ·  space: pause"
        }
    };

    let widths = [
        Constraint::Length(5),
        Constraint::Length(5),
        Constraint::Length(6),
        Constraint::Length(9),
        Constraint::Length(9),
        Constraint::Length(8),
        Constraint::Length(11),
        Constraint::Length(10),
        Constraint::Min(9),
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

fn draw_detail(frame: &mut ratatui::Frame, state: &TuiState, rows: &[ConvRow]) {
    let selected = state.selected.min(rows.len().saturating_sub(1));
    let Some(row) = rows.get(selected) else {
        return;
    };

    let area = centered_fixed(frame.area(), DETAIL_W, DETAIL_H);
    frame.render_widget(Clear, area);

    let head = Style::default().fg(ACCENT).add_modifier(Modifier::BOLD);
    let dim = Style::default().fg(Color::DarkGray);
    let mut lines: Vec<Line> = Vec::new();

    // Conversation summary
    let state_str = if row.active { "● active" } else { "✓ done" };
    lines.push(Line::from(Span::styled(
        format!(
            "  Conv #{} · slot [{}] · {} turns · {}",
            row.conv_id, row.slot, row.turn_count, state_str
        ),
        head,
    )));
    if let Some(ref t) = row.terminal {
        let color = match t {
            TerminalReason::ContextLimit => Color::Green,
            TerminalReason::MaxTurns => Color::Yellow,
            _ => Color::Red,
        };
        lines.push(Line::from(vec![
            Span::styled("  terminal: ", dim),
            Span::styled(t.to_string(), Style::default().fg(color)),
        ]));
    }
    {
        let mut summary = String::from("  ");
        if let Some(v) = row.avg_ttft_ms {
            summary.push_str(&format!("avg TTFT {v:.0}ms"));
        }
        if let Some(v) = row.avg_tps {
            summary.push_str(&format!("  ·  avg TPS {v:.1} tok/s"));
        }
        if !summary.trim().is_empty() {
            lines.push(Line::from(Span::styled(summary, dim)));
        }
    }
    lines.push(Line::from(""));

    // Turn table header
    lines.push(Line::from(Span::styled(
        format!(
            "  {:>4}  {:>10}  {:>10}  {:>7}  {:>7}  {:>8}  {:>7}  status",
            "turn", "prompt-tok", "compl-tok", "e2e", "ttft", "tpot", "tps"
        ),
        dim,
    )));
    lines.push(Line::from(Span::styled(
        format!("  {}", "─".repeat(72)),
        dim,
    )));

    // Fixed lead lines above the turn rows; turn `i` lands on line `lead + i`.
    let lead = lines.len();
    let sel_turn = state.detail_turn.min(row.turns.len().saturating_sub(1));
    let highlight = Style::default()
        .bg(Color::Indexed(238))
        .add_modifier(Modifier::BOLD);

    for (i, t) in row.turns.iter().enumerate() {
        let dash = "—";
        let selected = i == sel_turn;
        let status_span = if t.success {
            Span::styled("✓ ok", Style::default().fg(Color::Green))
        } else {
            let label = t.error.map_or("✗ error".to_string(), |e| format!("✗ {e}"));
            Span::styled(label, Style::default().fg(Color::Red))
        };
        // `▸` flags rows with content to open — a captured reply, or an error
        // to inspect on a failed turn.
        let openable = t.reply.as_deref().is_some_and(|r| !r.is_empty()) || !t.success;
        let marker = if openable {
            Span::styled("  ▸", dim)
        } else {
            Span::raw("")
        };
        let mut line = Line::from(vec![
            Span::raw(format!(
                "{} {:>4}  {:>10}  {:>10}  {:>7}  {:>7}  {:>8}  {:>7}  ",
                if selected { "›" } else { " " },
                t.turn_idx + 1,
                t.prompt_tokens
                    .map_or(dash.to_string(), |v| thousands(u64::from(v))),
                t.completion_tokens
                    .map_or(dash.to_string(), |v| thousands(u64::from(v))),
                format!("{:.2}s", t.e2e.as_secs_f64()),
                t.ttft
                    .map_or(dash.to_string(), |d| format!("{:.0}ms", d.as_secs_f64() * 1000.0)),
                t.tpot_ms.map_or(dash.to_string(), |v| format!("{v:.1}ms")),
                t.tps.map_or(dash.to_string(), |v| format!("{v:.1}")),
            )),
            status_span,
            marker,
        ]);
        if selected {
            line = line.style(highlight);
        }
        lines.push(line);
    }
    if row.turns.is_empty() {
        lines.push(Line::from(Span::styled("  (no turns recorded yet)", dim)));
    }

    // Keep the highlighted turn inside the viewport (header lines don't wrap at
    // this width, so line index == row index).
    let viewport = DETAIL_H.saturating_sub(2) as usize;
    let sel_line = lead + sel_turn;
    let scroll = sel_line.saturating_sub(viewport.saturating_sub(1)) as u16;

    let hint = if row.turns.is_empty() {
        "esc / q close".to_string()
    } else {
        "↑/↓ select · enter view · esc close".to_string()
    };
    let title = format!(" conv #{} detail  ·  {hint} ", row.conv_id);
    let para = Paragraph::new(lines)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(Span::styled(
                    title,
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                ))
                .padding(Padding::new(0, 1, 0, 0)),
        )
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    frame.render_widget(para, area);
}

/// Request/response drill-down for the selected turn. Rendered on top of the
/// detail modal; the body is built by `TuiState::turn_lines`.
fn draw_turn(frame: &mut ratatui::Frame, state: &TuiState, rows: &[ConvRow]) {
    let selected = state.selected.min(rows.len().saturating_sub(1));
    let Some(row) = rows.get(selected) else {
        return;
    };
    if row.turns.is_empty() {
        return;
    }
    let turn_idx = state.detail_turn.min(row.turns.len() - 1);

    let area = centered_fixed(frame.area(), TURN_W, TURN_H);
    frame.render_widget(Clear, area);

    let title = format!(
        " conv #{} · turn {}/{}  ·  ↑/↓ scroll · x expand · esc back ",
        row.conv_id,
        turn_idx + 1,
        row.turns.len()
    );
    let para = Paragraph::new(state.turn_lines())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(Span::styled(
                    title,
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                ))
                .padding(Padding::new(0, 1, 0, 0)),
        )
        .wrap(Wrap { trim: false })
        .scroll((state.turn_scroll, 0));
    frame.render_widget(para, area);
}

fn draw_footer(frame: &mut ratatui::Frame, area: Rect, state: &TuiState) {
    let text = match state.phase() {
        Phase::Paused => {
            let line = Line::from(vec![
                Span::styled(
                    " PAUSED ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    "  space/p resume  ·  ↑/↓ select  ·  enter detail  ·  q quit  ·  ? help",
                    Style::default().fg(Color::DarkGray),
                ),
            ]);
            frame.render_widget(Paragraph::new(line), area);
            return;
        }
        Phase::Replay => " [REPLAY]  q / esc quit  ·  ↑/↓ select  ·  enter detail  ·  ? help",
        Phase::Done => " [DONE]  q quit  ·  ↑/↓ select  ·  enter detail  ·  ? help",
        Phase::Live => " q quit  ·  space/p pause  ·  ↑/↓ select  ·  enter detail  ·  ? help",
    };
    let line = Line::from(vec![
        Span::styled(text, Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!("  ·  s sort: {}", state.sort.label()),
            Style::default().fg(ACCENT),
        ),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn draw_help(frame: &mut ratatui::Frame) {
    let area = centered_fixed(frame.area(), 72, 30);
    frame.render_widget(Clear, area);

    let key = |k: &'static str, d: &'static str| {
        Line::from(vec![
            Span::styled(
                format!("{k:<18}"),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::raw(d),
        ])
    };
    let term = |t: &'static str, d: &'static str| {
        Line::from(vec![
            Span::styled(
                format!("{t:<18}"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(d),
        ])
    };
    let section = |s: &'static str| {
        Line::from(Span::styled(
            s,
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ))
    };

    let lines = vec![
        section("KEYS"),
        key("↑/↓  j/k", "navigate conversations / turns / scroll request view"),
        key("g  G", "jump to top / bottom of the current list or view"),
        key("enter", "drill in: conversation → turn list → request/response"),
        key("x", "in request view: expand / collapse the full request payload"),
        key("esc", "step back out one level"),
        key("s", "cycle sort: recent → turns → TTFT → TPS"),
        key("space / p", "pause / resume dispatch and freeze metrics"),
        key("?", "toggle this help overlay"),
        key("q / Esc", "quit and restore terminal (report printed after)"),
        Line::from(""),
        section("METRICS"),
        term("TTFT", "time from request send to first content token (streaming)"),
        term("TPOT", "(e2e − TTFT) / (tokens − 1) — per-step decode latency"),
        term("TPS per-req", "completion_tokens / decode_window in tok/s"),
        term("TPS aggregate", "Σ completion_tokens / wall_clock (pause-excluded)"),
        term("p50 / p95 / p99", "percentiles across all successful turns"),
        Line::from(""),
        section("CONVERSATION STATES"),
        term("● active", "a slot is currently running turns"),
        term("✓ done", "conversation reached its terminal condition"),
        term("ctx-limit", "server refused: context-length overflow (normal)"),
        term("max-turns", "stopped by --max-turns cap"),
        term("error(…)", "network or API error terminated the conversation"),
        Line::from(""),
        Line::from(Span::styled(
            "  press  ?  ·  Esc  ·  or  q  to close",
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

// ── Small helpers ─────────────────────────────────────────────────────────────

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

fn ms_val(v: Option<f64>) -> String {
    v.map_or_else(|| "—".to_string(), |x| format!("{x:.0}ms"))
}

fn tps_val(v: Option<f64>) -> String {
    v.map_or_else(|| "—".to_string(), |x| format!("{x:.1}"))
}
