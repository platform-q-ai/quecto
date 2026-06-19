//! Application — the main TUI event loop.
//!
//! Wires all components together: terminal, UDS client, editor, chat,
//! spinner, footer, autocomplete, overlays, notifications, Kitty protocol,
//! signal handling, and extension management.

use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::infrastructure::client::{Client, Command, Event};
use crate::infrastructure::terminal::Terminal;
use crate::interface::component::Component;
use crate::interface::components::autocomplete::{Autocomplete, AutocompleteResult, SlashCommand};
use crate::interface::components::chat::Chat;
use crate::interface::components::chat::ChatEntry;
use crate::interface::components::editor::Editor;
use crate::interface::components::files_autocomplete::FilesAutocomplete;
use crate::interface::components::footer::Footer;
use crate::interface::components::model_selector::{ModelSelector, ModelSelectorResult};
use crate::interface::components::notification::{Notification, NotificationStack, NotifyLevel};
use crate::interface::components::select_list::{SelectItem, SelectList, SelectResult};
use crate::interface::components::spinner::Spinner;
use crate::interface::components::subagent_bar::{SubagentBar, SubagentRow};
use crate::interface::components::widget::WidgetContainer;
use crate::interface::components::workflow_bar;
use crate::interface::keys::{self, Key};
use crate::interface::kitty::KittyProtocol;
use crate::interface::overlay::OverlayStack;

/// Tick interval for spinner animation (~12fps).
const SPINNER_TICK: Duration = Duration::from_millis(80);

/// Lines scrolled per mouse wheel tick.
const MOUSE_SCROLL_LINES: usize = 3;

/// Maximum retry iterations for reassembling multi-fragment escape sequences.
/// Handles up to 5-fragment CSI splits on slow SSH/serial connections.
/// Total max wait = MAX_ESCAPE_RETRIES × escape_timeout (10ms) = 50ms.
const MAX_ESCAPE_RETRIES: usize = 5;

/// Built-in slash commands.
fn builtin_commands() -> Vec<SlashCommand> {
    vec![
        SlashCommand {
            name: "clear".into(),
            description: "Clear conversation history".into(),
        },
        SlashCommand {
            name: "quit".into(),
            description: "Exit TUI".into(),
        },
        SlashCommand {
            name: "exit".into(),
            description: "Exit TUI".into(),
        },
        SlashCommand {
            name: "help".into(),
            description: "Show keyboard shortcuts".into(),
        },
        SlashCommand {
            name: "hotkeys".into(),
            description: "Show keyboard shortcuts".into(),
        },
        SlashCommand {
            name: "new".into(),
            description: "Start a new session".into(),
        },
        SlashCommand {
            name: "session".into(),
            description: "Show session info".into(),
        },
        SlashCommand {
            name: "resume".into(),
            description: "Resume a persisted CLI session".into(),
        },
        SlashCommand {
            name: "model".into(),
            description: "Switch model".into(),
        },
        SlashCommand {
            name: "workflow".into(),
            description: "Show workflow status and hotkeys".into(),
        },
        SlashCommand {
            name: "workflow-auto".into(),
            description: "Toggle workflow auto-continue".into(),
        },
        SlashCommand {
            name: "workflow-nudge".into(),
            description: "Toggle workflow completion nudge".into(),
        },
    ]
}

/// Mouse selection anchor for click-and-drag text copy (#528).
#[derive(Debug, Clone, Copy)]
struct SelectionAnchor {
    col: u16,
    row: u16,
}

/// Active text selection (from mouse press to release) (#528).
#[derive(Debug, Clone)]
struct TextSelection {
    /// Where the mouse was pressed.
    start: SelectionAnchor,
    /// Current drag position (updated on mouse motion).
    end: SelectionAnchor,
}

/// Application state.
pub struct App {
    terminal: Terminal,
    client: Client,
    editor: Editor,
    chat: Chat,
    spinner: Option<Spinner>,
    footer: Footer,
    autocomplete: Autocomplete,
    files_autocomplete: FilesAutocomplete,
    notifications: NotificationStack,
    overlay_stack: OverlayStack,
    widgets_above: WidgetContainer,
    widgets_below: WidgetContainer,
    kitty: KittyProtocol,
    /// Agent run state with generation counter to prevent stale
    /// AgentEnd events from corrupting state after abort (#502).
    agent_state: AgentRunState,
    /// Whether the app should exit.
    should_exit: bool,
    /// Proper stdin buffer for escape sequence parsing.
    stdin_buffer: crate::interface::stdin_buffer::StdinBuffer,
    /// Whether the agent connection is still alive.
    agent_connected: bool,
    /// Current model name (from get_state), sanitized.
    current_model: Option<String>,
    /// Connected agent's own id (from get_state sessionKey); distinguishes its
    /// own workflow_state from descendants' forwarded events. None when unnamed.
    connected_agent_id: Option<String>,
    /// The model selector component (created on demand, pushed onto overlay stack).
    model_selector: Option<ModelSelector>,
    /// Session resume selector shown after `/resume` lists persisted sessions.
    resume_selector: Option<SelectList>,
    /// Rewind selector shown after idle double-Escape lists prior user turns.
    rewind_selector: Option<SelectList>,
    /// Last idle bare Escape used to detect double-Escape for rewind.
    last_idle_escape: Option<tokio::time::Instant>,
    /// Locally-issued get_messages id for opening the rewind selector.
    pending_rewind_open_id: Option<String>,
    /// Locally-issued rewind_to id awaiting acknowledgement.
    pending_rewind_apply_id: Option<String>,
    /// Monotonic client-local sequence for rewind correlation ids.
    rewind_request_seq: u64,
    /// Client-side subagent state for immediate bar updates (#525).
    /// Updated from tool events (spawn/agent_cmd) and server pushes.
    /// Entries track expiry timestamps for auto-removal (#540).
    subagent_local: std::collections::BTreeMap<String, TrackedSubagent>,
    /// Animation frame for the subagent spinner, advanced on each spinner tick.
    subagent_frame: usize,
    /// The sub-agent the parent is currently blocked on via `agent_cmd await`,
    /// if any. Rendered as a per-row "awaiting" indicator instead of a shared
    /// spinner line.
    awaited_agent_id: Option<String>,
    /// Diagnostic: when `QUECTO_TUI_RENDER_LOG` is set, every frame is appended
    /// (ANSI-stripped) to this file for frame-by-frame replay.
    render_log_path: Option<String>,
    /// Active mouse text selection (#528).
    selection: Option<TextSelection>,
    /// Workflow header bar state (#563).
    workflow_bar: workflow_bar::WorkflowBarState,
    /// Mirror of core workflow auto-continue state, toggled through UDS.
    workflow_auto_continue: bool,
    /// Mirror of core workflow completion-nudge state, toggled through UDS.
    workflow_completion_nudge: bool,
    /// Last observed git branch shown in the footer.
    git_branch: Option<String>,
    /// Repository root used for git branch polling.
    git_repo: Option<PathBuf>,
    /// Last rendered lines (for extracting selected text from the buffer).
    last_rendered_lines: Vec<String>,
    /// Whether we've already requested session stats as a fallback to learn
    /// the real context window for the current session/model.
    context_stats_requested: bool,
}

impl App {
    pub fn new(terminal: Terminal, client: Client) -> Self {
        let mut footer = Footer::new();
        let git_repo = std::env::current_dir().ok();
        let git_branch = git_repo.as_deref().and_then(app_git::read_git_branch_from);
        footer.set_git_branch(git_branch.clone());

        Self {
            terminal,
            client,
            editor: Editor::new(),
            chat: Chat::new(),
            spinner: None,
            footer,
            autocomplete: Autocomplete::new(builtin_commands(), 8),
            files_autocomplete: FilesAutocomplete::new(8),
            notifications: NotificationStack::new(),
            overlay_stack: OverlayStack::new(),
            widgets_above: WidgetContainer::new(),
            widgets_below: WidgetContainer::new(),
            kitty: KittyProtocol::new(),
            agent_state: AgentRunState::new(),
            should_exit: false,
            stdin_buffer: crate::interface::stdin_buffer::StdinBuffer::new(),
            agent_connected: true,
            current_model: None,
            connected_agent_id: None,
            model_selector: None,
            resume_selector: None,
            rewind_selector: None,
            last_idle_escape: None,
            pending_rewind_open_id: None,
            pending_rewind_apply_id: None,
            rewind_request_seq: 0,
            subagent_local: std::collections::BTreeMap::new(),
            subagent_frame: 0,
            awaited_agent_id: None,
            render_log_path: std::env::var("QUECTO_TUI_RENDER_LOG").ok(),
            selection: None,
            workflow_bar: workflow_bar::WorkflowBarState::default(),
            workflow_auto_continue: false,
            workflow_completion_nudge: false,
            git_branch,
            git_repo,
            last_rendered_lines: Vec::new(),
            context_stats_requested: false,
        }
    }

    fn apply_git_branch(&mut self, branch: Option<String>) -> bool {
        if branch == self.git_branch {
            return false;
        }
        self.git_branch = branch.clone();
        self.footer.set_git_branch(branch);
        true
    }

    fn start_git_branch_refresh(&self, tx: &mpsc::Sender<Option<String>>, in_flight: &mut bool) {
        if *in_flight {
            return;
        }
        let Some(repo) = self.git_repo.clone() else {
            return;
        };
        *in_flight = true;
        let tx = tx.clone();
        tokio::task::spawn_blocking(move || {
            let branch = app_git::read_git_branch_from(&repo);
            let _ = tx.blocking_send(branch);
        });
    }
}

#[path = "app_event_loop.rs"]
mod app_event_loop;
#[path = "app_events.rs"]
mod app_events;
#[path = "app_git.rs"]
mod app_git;
#[path = "app_methods.rs"]
mod app_methods;
#[path = "app_response.rs"]
mod app_response;
#[path = "app_rewind.rs"]
mod app_rewind;
#[path = "app_subagents.rs"]
mod app_subagents;

/// Maximum bytes for OSC 52 clipboard payload (100 KiB before base64 encoding).
/// Some terminals (e.g. tmux) have a ~64 KiB limit; this cap prevents
/// sending excessively large payloads that would be silently dropped.
const MAX_CLIPBOARD_BYTES: usize = 100 * 1024;

/// Copy text to the system clipboard using OSC 52 escape sequence (#528).
///
/// OSC 52 is supported by most modern terminals (kitty, iTerm2, WezTerm,
/// Alacritty, tmux, etc.) and works over SSH without needing xclip/xsel.
/// Falls back silently if the terminal doesn't support it.
fn copy_to_clipboard(text: &str) {
    use std::io::Write;
    // Cap payload size to avoid overwhelming terminals with large selections.
    let capped = if text.len() > MAX_CLIPBOARD_BYTES {
        &text[..MAX_CLIPBOARD_BYTES]
    } else {
        text
    };
    // Base64-encode the text for OSC 52.
    // OSC 52 format: \x1b]52;c;<base64>\x07
    let encoded = base64_encode(capped.as_bytes());
    let osc = format!("\x1b]52;c;{}\x07", encoded);
    let _ = std::io::stdout().write_all(osc.as_bytes());
    let _ = std::io::stdout().flush();
}

/// Simple base64 encoder (no external dependency).
fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        result.push(ALPHABET[((triple >> 18) & 0x3F) as usize] as char);
        result.push(ALPHABET[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(ALPHABET[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(ALPHABET[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

/// Normalize a selection into (start_row, start_col, end_row, end_col) order (#546).
/// Ensures start ≤ end regardless of drag direction.
fn selection_range(sel: &TextSelection) -> (u16, u16, u16, u16) {
    let (sr, sc, er, ec) = if sel.start.row < sel.end.row
        || (sel.start.row == sel.end.row && sel.start.col <= sel.end.col)
    {
        (sel.start.row, sel.start.col, sel.end.row, sel.end.col)
    } else {
        (sel.end.row, sel.end.col, sel.start.row, sel.start.col)
    };
    (sr, sc, er, ec)
}

/// Apply mouse selection highlight to rendered lines (#546).
fn apply_selection_highlight(selection: &Option<TextSelection>, lines: &mut [String]) {
    let Some(sel) = selection else { return };
    let (sr, sc, er, ec) = selection_range(sel);
    for row_idx in sr..=er {
        if (row_idx as usize) < lines.len() {
            let line_start = if row_idx == sr { sc } else { 0 };
            let line_end = if row_idx == er {
                ec
            } else {
                crate::interface::utils::visible_width(&lines[row_idx as usize]) as u16
            };
            lines[row_idx as usize] =
                apply_line_highlight(&lines[row_idx as usize], line_start, line_end);
        }
    }
}

/// Apply reverse-video highlighting to a range of visible columns in a line (#546).
///
/// Takes a rendered line (may contain ANSI escapes) and highlights columns
/// `start_col..end_col` (0-indexed, exclusive end) by wrapping visible chars
/// in that range with `\x1b[7m` (reverse) and `\x1b[27m` (reverse off).
fn apply_line_highlight(line: &str, start_col: u16, end_col: u16) -> String {
    if start_col >= end_col {
        return line.to_string();
    }
    let mut result = String::with_capacity(line.len() + 20);
    let mut vis_col: u16 = 0;
    let mut in_esc = false;
    let mut in_osc = false;
    let mut highlighted = false;
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let ch = chars[i];
        // Pass through ANSI escape sequences without counting columns.
        if in_osc {
            result.push(ch);
            if ch == '\x07' {
                in_osc = false;
            } else if ch == '\x1b' && i + 1 < chars.len() && chars[i + 1] == '\\' {
                result.push(chars[i + 1]);
                i += 2;
                in_osc = false;
                continue;
            }
            i += 1;
            continue;
        }
        if in_esc {
            result.push(ch);
            if ch.is_ascii_alphabetic() || ch == '~' {
                in_esc = false;
            }
            i += 1;
            continue;
        }
        if ch == '\x1b' {
            result.push(ch);
            in_osc = i + 1 < chars.len() && chars[i + 1] == ']';
            if !in_osc {
                in_esc = true;
            }
            i += 1;
            continue;
        }
        // Visible character — apply highlight bracketing.
        if vis_col == start_col && !highlighted {
            result.push_str("\x1b[7m");
            highlighted = true;
        }
        result.push(ch);
        vis_col += 1;
        if vis_col == end_col && highlighted {
            result.push_str("\x1b[27m");
            highlighted = false;
        }
        i += 1;
    }
    if highlighted {
        result.push_str("\x1b[27m");
    }
    result
}

/// Strip ANSI escape sequences from a string to get visible text.
fn strip_ansi_for_selection(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_escape = false;
    let mut in_osc = false;
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if in_osc {
            // OSC ends with BEL (\x07) or ST (\x1b\\)
            if chars[i] == '\x07' {
                in_osc = false;
            } else if chars[i] == '\x1b' && i + 1 < chars.len() && chars[i + 1] == '\\' {
                in_osc = false;
                i += 1;
            }
        } else if in_escape {
            if chars[i].is_ascii_alphabetic() || chars[i] == '~' {
                in_escape = false;
            }
        } else if chars[i] == '\x1b' {
            if i + 1 < chars.len() && chars[i + 1] == ']' {
                in_osc = true;
                i += 1;
            } else {
                in_escape = true;
            }
        } else {
            result.push(chars[i]);
        }
        i += 1;
    }
    result
}

/// Truncate tool arguments for spinner display.
fn truncate_args(args: &str) -> String {
    let clean: String = args
        .chars()
        .filter(|&c| c >= ' ' && c != '\u{007F}')
        .collect();
    if clean.chars().count() > 40 {
        let s: String = clean.chars().take(37).collect();
        format!("{}...", s)
    } else {
        clean
    }
}

// ── Agent state machine (extracted for testability) ───────────────────────

/// Tracks agent running state with abort-awareness to prevent
/// stale AgentEnd events from corrupting state after abort (#502).
///
/// The agent protocol doesn't include generation/request IDs in events,
/// so we track the number of pending aborts. When an AgentEnd arrives
/// after an abort, we consume one pending abort and ignore the event
/// instead of setting running=false (which would kill a new run).
///
/// All state transitions go through methods — `running` is private
/// to enforce invariants.
#[derive(Debug)]
pub(crate) struct AgentRunState {
    /// Whether the agent is currently processing.
    running: bool,
    /// Number of aborted runs whose AgentEnd events haven't arrived yet.
    /// Capped at 8 to prevent unbounded growth if the agent misbehaves.
    pending_aborts: u32,
    /// Monotonically increasing generation counter (for diagnostics).
    generation: u64,
}

/// Safety cap on pending aborts. In practice, `start()` clears
/// `pending_aborts` so this is only hit if `abort()` is called
/// repeatedly without intervening `start()` (shouldn't happen).
const MAX_PENDING_ABORTS: u32 = 8;

impl AgentRunState {
    pub fn new() -> Self {
        Self {
            running: false,
            pending_aborts: 0,
            generation: 0,
        }
    }

    /// Whether the agent is currently processing.
    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Start a new agent run. Increments generation and clears any
    /// pending aborts — once a new AgentStart arrives, stale AbortEnd
    /// events from previous runs are irrelevant (#506).
    pub fn start(&mut self) {
        self.generation += 1;
        self.running = true;
        self.pending_aborts = 0;
    }

    /// Handle an abort. Increments pending_aborts so the stale AgentEnd
    /// from this run will be consumed without affecting a new run.
    /// Sets running=false for UI purposes (stop showing active state).
    pub fn abort(&mut self) {
        if self.running {
            if self.pending_aborts < MAX_PENDING_ABORTS {
                self.pending_aborts += 1;
            }
            self.running = false;
        }
    }

    /// Handle an AgentEnd event.
    ///
    /// If there are pending aborts, consumes one and returns false
    /// (stale event — don't update UI). Otherwise, sets running=false
    /// and returns true (current run ended normally).
    pub fn end(&mut self) -> bool {
        if self.pending_aborts > 0 {
            self.pending_aborts -= 1;
            // Stale AgentEnd from an aborted run — ignore.
            false
        } else {
            self.running = false;
            true
        }
    }

    /// Hard reset — used when the agent disconnects or errors out.
    /// Clears all state unconditionally.
    pub fn reset(&mut self) {
        self.running = false;
        self.pending_aborts = 0;
    }
}

/// Result of a Ctrl+C key press (#536).
#[derive(Debug, PartialEq, Eq)]
enum CtrlCAction {
    /// Clear the editor text (and dismiss autocomplete).
    ClearEditor,
    /// Abort the running agent.
    AbortAgent,
    /// Nothing to do.
    Noop,
}

/// Decide what Ctrl+C should do based on agent and editor state (#536).
///
/// Priority: clear editor text first; only abort if editor is already empty.
fn ctrl_c_action(agent_running: bool, editor_empty: bool) -> CtrlCAction {
    if !editor_empty {
        CtrlCAction::ClearEditor
    } else if agent_running {
        CtrlCAction::AbortAgent
    } else {
        CtrlCAction::Noop
    }
}

/// Whether a tool is subagent-related (spawn or agent_cmd).
///
/// Used for subagent state refresh — both tools trigger a `GetSubagents`
/// request after completion.
fn is_subagent_tool(tool_name: &str) -> bool {
    tool_name == "spawn" || tool_name == "agent_cmd"
}

/// Whether to suppress tool output boxes in the chat area (#538).
///
/// `spawn` output is always suppressed (status bar provides visibility).
/// `agent_cmd` mutations (prompt, steer, abort) are suppressed — the
/// status bar shows subagent activity. Query commands (get_state,
/// get_messages_tail, get_session_stats, etc.) are shown so the user
/// can inspect results.
/// Status string for exited subagents — used in multiple comparisons (#540).
const STATUS_EXITED: &str = "exited";

/// Grace period before exited subagent bars are auto-removed (#540).
const EXITED_SUBAGENT_GRACE: Duration = Duration::from_secs(5);

/// Strip control characters from an agent_id for safe use as a map key.
fn sanitize_workflow_status_text(text: &str, max_chars: usize) -> String {
    let clean: String = text
        .chars()
        .filter(|c| !c.is_control())
        .take(max_chars)
        .collect();
    if text.chars().filter(|c| !c.is_control()).count() > max_chars {
        format!("{clean}…")
    } else {
        clean
    }
}

fn sanitize_agent_id(id: &str) -> String {
    id.chars().filter(|c| !c.is_control()).collect()
}

/// Subagent entry with optional expiry timestamp (#540).
#[derive(Debug, Clone)]
struct TrackedSubagent {
    info: crate::infrastructure::client::SubagentInfoEvent,
    /// When the subagent was first observed (for the elapsed-time display).
    started_at: tokio::time::Instant,
    /// When the subagent last stopped being active (idle/error/exited), used to
    /// freeze the elapsed-time display. `None` while active. Without this, an
    /// idle agent's timer keeps ticking until the *last* sibling goes idle
    /// (the animation tick runs while any agent is active).
    stopped_at: Option<tokio::time::Instant>,
    /// When the subagent entered the "exited" state. `None` if not exited. Kept
    /// distinct from `stopped_at` because GC grace counts from exit, while the
    /// timer freezes from when the agent first went idle.
    exited_at: Option<tokio::time::Instant>,
}

/// Whether a subagent status counts as "actively running" for the timer.
fn subagent_status_is_active(status: &str) -> bool {
    matches!(status, "starting" | "running")
}

impl TrackedSubagent {
    fn new(info: crate::infrastructure::client::SubagentInfoEvent) -> Self {
        let now = tokio::time::Instant::now();
        let active = subagent_status_is_active(&info.status);
        let exited_at = if info.status == STATUS_EXITED {
            Some(now)
        } else {
            None
        };
        Self {
            info,
            started_at: now,
            stopped_at: if active { None } else { Some(now) },
            exited_at,
        }
    }

    /// Seconds the agent was actively running, frozen once it goes idle/exits.
    fn elapsed_secs(&self, now: tokio::time::Instant) -> u64 {
        let end = self.stopped_at.unwrap_or(now);
        end.saturating_duration_since(self.started_at).as_secs()
    }

    /// Update the info, freezing the timer when the agent stops being active and
    /// recording exited_at on transition to "exited".
    fn update_info(&mut self, mut new_info: crate::infrastructure::client::SubagentInfoEvent) {
        // Preserve last-known workflow + parent_id when an update omits them
        // (get_subagents carries neither, and would otherwise erase the n/n).
        if new_info.workflow.is_none() {
            new_info.workflow = self.info.workflow.clone();
        }
        if new_info.parent_id.is_none() {
            new_info.parent_id = self.info.parent_id.clone();
        }
        let now = tokio::time::Instant::now();
        if subagent_status_is_active(&new_info.status) {
            // Resumed work — let the timer run again.
            self.stopped_at = None;
        } else if self.stopped_at.is_none() {
            // First transition into a stopped state — freeze the timer here.
            self.stopped_at = Some(now);
        }
        if new_info.status == STATUS_EXITED && self.exited_at.is_none() {
            self.exited_at = Some(now);
        } else if new_info.status != STATUS_EXITED {
            self.exited_at = None;
        }
        self.info = new_info;
    }
}

/// Remove exited subagents whose grace period has elapsed (#540).
/// Returns `true` if any entries were removed.
///
/// While any sibling is still active, finished agents are kept on screen so the
/// panel doesn't shrink mid-batch and jolt the chat above it — reclamation is
/// deferred until the whole batch is quiescent (the panel then grows once and
/// clears once instead of oscillating as agents come and go).
fn gc_exited_subagents(
    map: &mut std::collections::BTreeMap<String, TrackedSubagent>,
    now: tokio::time::Instant,
    grace: Duration,
) -> bool {
    if map
        .values()
        .any(|entry| subagent_status_is_active(&entry.info.status))
    {
        return false;
    }
    let mut removed = false;
    map.retain(|_, entry| {
        if let Some(exited_at) = entry.exited_at {
            let keep = now.saturating_duration_since(exited_at) < grace;
            if !keep {
                removed = true;
            }
            keep
        } else {
            true
        }
    });
    removed
}

#[cfg(test)]
#[path = "app_subagent_selection_tests.rs"]
mod subagent_selection_tests;
#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "tui_harness.rs"]
mod tui_harness;
#[cfg(test)]
#[path = "tui_harness_tests.rs"]
mod tui_harness_tests;
