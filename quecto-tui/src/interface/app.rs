//! Application — the main TUI event loop.
//!
//! Wires all components together: terminal, UDS client, editor, chat,
//! spinner, footer, autocomplete, overlays, notifications, Kitty protocol,
//! signal handling, and extension management.

use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use app_selection::TextSelection;

use tokio::sync::mpsc;

use crate::infrastructure::client::{Client, Command, Event};
use crate::infrastructure::render::DiffRenderer;
use crate::infrastructure::terminal::Terminal;
use crate::infrastructure::workspace_files::list_workspace_files;
use crate::interface::component::Component;
use crate::interface::components::autocomplete::{Autocomplete, AutocompleteResult, SlashCommand};
use crate::interface::components::chat::Chat;
use crate::interface::components::chat::ChatEntry;
use crate::interface::components::editor::Editor;
use crate::interface::components::files_autocomplete::FilesAutocomplete;
use crate::interface::components::footer::Footer;
use crate::interface::components::model_selector::{
    ModelEntry, ModelSelector, ModelSelectorResult,
};
use crate::interface::components::notification::{Notification, NotificationStack, NotifyLevel};
use crate::interface::components::select_list::{SelectItem, SelectList, SelectResult};
use crate::interface::components::spinner::Spinner;
use crate::interface::components::subagent_bar::{SubagentBar, SubagentRow};
use crate::interface::components::widget::WidgetContainer;
use crate::interface::components::workflow_bar;
use crate::interface::keys::{self, Key};
use crate::interface::kitty::KittyProtocol;

const SPINNER_TICK: Duration = Duration::from_millis(80);
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

/// Application state.
pub struct App {
    terminal: Terminal,
    renderer: DiffRenderer<std::io::Stdout>,
    client: Client,
    editor: Editor,
    chat: Chat,
    spinner: Option<Spinner>,
    footer: Footer,
    autocomplete: Autocomplete,
    files_autocomplete: FilesAutocomplete,
    notifications: NotificationStack,
    widgets_above: WidgetContainer,
    kitty: KittyProtocol,
    agent_state: AgentRunState,
    should_exit: bool,
    stdin_buffer: crate::interface::stdin_buffer::StdinBuffer,
    agent_connected: bool,
    current_model: Option<String>,
    /// Connected agent's own id (from get_state sessionKey); distinguishes its
    /// own workflow_state from descendants' forwarded events. None when unnamed.
    connected_agent_id: Option<String>,
    /// The model selector component (created on demand, pushed onto overlay stack).
    model_selector: Option<ModelSelector>,
    model_registry: (Vec<ModelEntry>, bool),
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
    /// Internal notifications for asynchronous command-send failures.
    command_send_failure_tx: mpsc::Sender<CommandSendFailure>,
    command_send_failure_rx: mpsc::Receiver<CommandSendFailure>,
    /// Per-sub-agent session views, keyed by agent id (#800). The master is not
    /// in this map — it is the top-level `self.chat`/`self.workflow_bar`/etc.
    /// and is represented by `active_agent_id == None`. Sessions are retained
    /// after a sub-agent exits so the user can keep viewing its last session,
    /// bounded by `MAX_RETAINED_SESSIONS`.
    sessions: std::collections::BTreeMap<String, SessionView>,
    /// Insertion order of session ids, for bounded retention eviction (#800).
    session_order: Vec<String>,
    /// The agent whose session is shown in the main body. `None` = master.
    active_agent_id: Option<String>,
    /// Left-panel selection cursor over the flattened (master + tree) rows.
    panel_nav: crate::interface::components::list_navigator::ListNavigator,
    /// Fan-in for events from the active sub-agent's direct connection (#800).
    /// Each item is `(agent_id, event)`; routed into that agent's session.
    subagent_event_tx: mpsc::Sender<(String, Event)>,
    subagent_event_rx: mpsc::Receiver<(String, Event)>,
    /// The agent id of the currently-open connect-on-select connection and a
    /// handle to abort its forwarding task on deselect/teardown (#800).
    active_conn: Option<(String, tokio::task::JoinHandle<()>)>,
    /// Outbound command channel to the active sub-agent's own UDS connection
    /// (#802). Lets the editor's send/abort path steer the selected sub-agent —
    /// its dispatch loop queues the prompt until its current turn ends. `None`
    /// when the master is active or the child's socket is unknown.
    active_subagent_cmd_tx: Option<(String, mpsc::Sender<Command>)>,
    /// Which pane has keyboard focus: the editor or the side panel (#802).
    focus: Focus,
    /// When the TUI session started — drives the Master row's uptime timer (#820).
    started_at: tokio::time::Instant,
}

/// Per-sub-agent session state for the multi-session UI (#800/#802). Holds the
/// child's own chat transcript and its OWN workflow/phase bar, accumulated from
/// its direct live stream, plus a running flag driving the per-session working
/// indicator. The editor and overlays stay single-instance on `App`.
pub(crate) struct SessionView {
    chat: Chat,
    /// The child's own workflow/phase bar, fed by its forwarded `workflow_state`
    /// events so a selected sub-agent renders the same bar the master would.
    workflow_bar: workflow_bar::WorkflowBarState,
    /// Whether the child is mid-turn — drives a per-session working indicator so
    /// a steered sub-agent never looks dead while it processes a queued prompt.
    running: bool,
    /// The child's OWN status footer — context-window / cost / model gauges, fed
    /// by its forwarded `get_state` / `turn_end` / session-stats events so a
    /// selected sub-agent shows ITS usage, not the master's (#805).
    footer: Footer,
}

impl SessionView {
    fn new(git_branch: Option<String>) -> Self {
        let mut footer = Footer::new();
        footer.set_git_branch(git_branch);
        Self {
            chat: Chat::new(),
            workflow_bar: workflow_bar::WorkflowBarState::default(),
            running: false,
            footer,
        }
    }
}

/// Which pane currently has keyboard focus (#802). The editor (`Input`) is the
/// default; `Tab` toggles to the side `Panel` when sub-agents are listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Focus {
    Input,
    Panel,
}

/// Width of the persistent left sub-agent panel (#800).
const SUBAGENT_PANEL_WIDTH: usize = 26;

/// Maximum retained sub-agent sessions before the oldest non-active is evicted.
const MAX_RETAINED_SESSIONS: usize = 16;

struct CommandSendFailure {
    command_kind: &'static str,
    error: String,
}

impl App {
    pub fn new(terminal: Terminal, client: Client) -> Self {
        let mut footer = Footer::new();
        let git_repo = std::env::current_dir().ok();
        let git_branch = git_repo.as_deref().and_then(app_git::read_git_branch_from);
        footer.set_git_branch(git_branch.clone());
        let (command_send_failure_tx, command_send_failure_rx) = mpsc::channel(16);
        let (subagent_event_tx, subagent_event_rx) = mpsc::channel(256);

        Self {
            terminal,
            renderer: DiffRenderer::new(std::io::stdout()),
            client,
            editor: Editor::new(),
            chat: Chat::new(),
            spinner: None,
            footer,
            autocomplete: Autocomplete::new(builtin_commands(), 8),
            files_autocomplete: FilesAutocomplete::new(8),
            notifications: NotificationStack::new(),
            widgets_above: WidgetContainer::new(),
            kitty: KittyProtocol::new(),
            agent_state: AgentRunState::new(),
            should_exit: false,
            stdin_buffer: crate::interface::stdin_buffer::StdinBuffer::new(),
            agent_connected: true,
            current_model: None,
            connected_agent_id: None,
            model_selector: None,
            model_registry: (Vec::new(), false),
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
            command_send_failure_tx,
            command_send_failure_rx,
            sessions: std::collections::BTreeMap::new(),
            session_order: Vec::new(),
            active_agent_id: None,
            panel_nav: crate::interface::components::list_navigator::ListNavigator::new(),
            subagent_event_tx,
            subagent_event_rx,
            active_conn: None,
            active_subagent_cmd_tx: None,
            focus: Focus::Input,
            started_at: tokio::time::Instant::now(),
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

    fn start_files_autocomplete_load(&self, tx: &mpsc::Sender<Vec<String>>, in_flight: &mut bool) {
        if *in_flight {
            return;
        }
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        *in_flight = true;
        let tx = tx.clone();
        tokio::task::spawn_blocking(move || {
            let files = list_workspace_files(&cwd);
            let _ = tx.blocking_send(files);
        });
    }

    fn refresh_files_autocomplete_from_editor(&mut self) {
        let line = self.editor.current_line().to_string();
        let col = self.editor.cursor_col();
        self.files_autocomplete.update(&line, col);
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
#[path = "app_models.rs"]
mod app_models;
#[path = "app_response.rs"]
mod app_response;
#[path = "app_rewind.rs"]
mod app_rewind;
#[path = "app_selection.rs"]
mod app_selection;
#[path = "app_subagent_panel.rs"]
mod app_subagent_panel;
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
fn copy_to_clipboard(text: &str) -> std::io::Result<()> {
    let mut stdout = std::io::stdout();
    write_osc52_clipboard_sequence(text, &mut stdout)
}

pub fn write_osc52_clipboard_sequence(text: &str, writer: &mut impl Write) -> std::io::Result<()> {
    // Cap payload size to avoid overwhelming terminals with large selections.
    let bytes = text.as_bytes();
    let capped = &bytes[..bytes.len().min(MAX_CLIPBOARD_BYTES)];
    // Base64-encode the text for OSC 52.
    // OSC 52 format: \x1b]52;c;<base64>\x07
    let encoded = base64_encode(capped);
    let osc = format!("\x1b]52;c;{}\x07", encoded);
    writer.write_all(osc.as_bytes())?;
    writer.flush()
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

/// Strip ANSI escape sequences from a string to get visible text.
fn strip_ansi_for_selection(s: &str) -> String {
    crate::interface::ansi::strip_ansi(s)
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
    let (clean, truncated) = crate::interface::ansi::sanitize_control_truncated(text, max_chars);
    if truncated {
        format!("{clean}…")
    } else {
        clean
    }
}

fn sanitize_agent_id(id: &str) -> String {
    crate::interface::ansi::sanitize_control(id)
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
        let exited_at = (info.status == STATUS_EXITED).then_some(now);
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
    map.retain(|_, entry| match entry.exited_at {
        Some(exited_at) => {
            let keep = now.saturating_duration_since(exited_at) < grace;
            if !keep {
                removed = true;
            }
            keep
        }
        None => true,
    });
    removed
}

#[cfg(test)]
#[path = "app_clipboard_tests.rs"]
mod app_clipboard_tests;
#[cfg(test)]
#[path = "app_cov_tests.rs"]
mod app_cov_tests;
#[cfg(test)]
#[path = "app_event_loop_tests.rs"]
mod app_event_loop_tests;
#[cfg(test)]
#[path = "app_methods_tests.rs"]
mod app_methods_tests;
#[cfg(test)]
#[path = "app_rewind_response_tests.rs"]
mod app_rewind_response_tests;
#[cfg(test)]
#[path = "app_selection_tests.rs"]
mod app_selection_tests;
#[cfg(test)]
#[path = "app_subagent_first_tests.rs"]
mod app_subagent_first_tests;
#[cfg(test)]
#[path = "app_subagent_panel_tests.rs"]
mod app_subagent_panel_tests;
#[cfg(test)]
#[path = "app_subagents_tests.rs"]
mod app_subagents_tests;
#[cfg(test)]
#[path = "app_chat_session_tests.rs"]
mod chat_session_tests;
#[cfg(test)]
#[path = "app_focus_parity_tests.rs"]
mod focus_parity_tests;
#[cfg(test)]
#[path = "app_subagent_selection_tests.rs"]
mod subagent_selection_tests;
#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
// The headless render harness is also exposed (read-only driving surface) to the
// workspace `bdd` integration target via the `test-harness` feature (#805).
#[cfg(any(test, feature = "test-harness"))]
#[path = "tui_harness.rs"]
pub mod tui_harness;
#[cfg(test)]
#[path = "tui_harness_tests.rs"]
mod tui_harness_tests;
