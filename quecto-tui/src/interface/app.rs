use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

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
use crate::interface::components::workflow_bar;
use crate::interface::keys::{self, Key};
use crate::interface::kitty::KittyProtocol;
use app_selection::TextSelection;
use tokio::sync::mpsc;

const SPINNER_TICK: Duration = Duration::from_millis(80);
pub(super) const STREAM_RENDER_INTERVAL: Duration = Duration::from_millis(33);
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
    /// The master agent's own session, modeled as just another [`SessionView`]
    /// (#828) so render/input share ONE active-session path with sub-agents
    /// (`active_agent_id == None` selects this). Only `spinner`/`agent_state`
    /// stay master-local; sub-agents derive `running` from forwarded events.
    master_session: SessionView,
    spinner: Option<Spinner>,
    autocomplete: Autocomplete,
    files_autocomplete: FilesAutocomplete,
    notifications: NotificationStack,
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
    /// in this map — it is the top-level `self.master_session.chat`/`self.master_session.workflow_bar`/etc. and
    /// is `active_agent_id == None`. Sessions are retained after a sub-agent exits
    /// so its last session stays viewable, bounded by `MAX_RETAINED_SESSIONS`.
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

/// Per-session state for the multi-session UI (#800/#802). The master (#828)
/// and every sub-agent are modeled identically: own chat transcript, own
/// workflow/phase bar, a running flag driving the per-session working indicator,
/// and own footer. The editor and overlays stay single-instance on `App`.
pub(crate) struct SessionView {
    chat: Chat,
    /// The child's own workflow/phase bar, fed by its forwarded `workflow_state`.
    workflow_bar: workflow_bar::WorkflowBarState,
    /// Whether the child is mid-turn — drives a per-session working indicator.
    running: bool,
    /// This session's OWN status footer (context-window / cost / model gauges, #805).
    footer: Footer,
    /// Grandchild completion notes buffered while mid-turn; flushed at idle (#816).
    deferred_subagent_notes: std::collections::VecDeque<String>,
    /// Whether the history backfill was applied (#828) — guards re-delivery.
    history_backfilled: bool,
    /// Whether this session's own stream has reported run-state yet; until it has
    /// `active_subagent_running` trusts the tracked status not `running` (#834).
    observed_run_state: bool,
}

impl SessionView {
    fn new(git_branch: Option<String>) -> Self {
        let mut footer = Footer::new();
        footer.set_git_branch(git_branch);
        Self::with_footer(footer)
    }

    /// Build a session around a pre-configured footer — used for both sub-agent
    /// and master (#828) sessions, so all are constructed identically.
    fn with_footer(footer: Footer) -> Self {
        Self {
            chat: Chat::new(),
            workflow_bar: workflow_bar::WorkflowBarState::default(),
            running: false,
            footer,
            deferred_subagent_notes: std::collections::VecDeque::new(),
            history_backfilled: false,
            observed_run_state: false,
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

/// Width of the persistent left sub-agent panel (#800); room for names + a bar.
const SUBAGENT_PANEL_WIDTH: usize = 30;

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
            master_session: SessionView::with_footer(footer),
            spinner: None,
            autocomplete: Autocomplete::new(builtin_commands(), 8),
            files_autocomplete: FilesAutocomplete::new(8),
            notifications: NotificationStack::new(),
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

    pub(super) fn apply_git_branch(&mut self, branch: Option<String>) -> bool {
        if branch == self.git_branch {
            return false;
        }
        self.git_branch = branch.clone();
        self.master_session.footer.set_git_branch(branch);
        true
    }

    pub(super) fn start_git_branch_refresh(
        &self,
        tx: &mpsc::Sender<Option<String>>,
        in_flight: &mut bool,
    ) {
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
pub const GIT_BRANCH_POLL_INTERVAL: std::time::Duration = app_git::GIT_BRANCH_POLL_INTERVAL;
#[path = "app_idle_efficiency.rs"]
mod app_idle_efficiency;
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
#[path = "app_subagent_state.rs"]
mod app_subagent_state;
#[path = "app_submit.rs"]
mod app_submit;
use app_subagent_state::{
    TrackedSubagent, gc_exited_subagents, next_exited_subagent_gc_deadline,
    subagent_status_is_active,
};
#[path = "app_subagent_stream.rs"]
mod app_subagent_stream;
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
#[path = "app_git_tests.rs"]
mod app_git_tests;
#[cfg(test)]
#[path = "app_idle_efficiency_tests.rs"]
mod app_idle_efficiency_tests;
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
#[path = "app_streaming_stability_tests.rs"]
mod app_streaming_stability_tests;
#[cfg(test)]
#[path = "app_subagent_first_tests.rs"]
mod app_subagent_first_tests;
#[cfg(test)]
#[path = "app_subagent_panel_observer_tests.rs"]
mod app_subagent_panel_observer_tests;
#[cfg(test)]
#[path = "app_subagent_panel_tests.rs"]
mod app_subagent_panel_tests;
#[cfg(test)]
#[path = "app_subagent_workflow_sticky_tests.rs"]
mod app_subagent_workflow_sticky_tests;
#[cfg(test)]
#[path = "app_subagents_tests.rs"]
mod app_subagents_tests;
#[cfg(test)]
#[path = "app_workflow_box_width_tests.rs"]
mod app_workflow_box_width_tests;
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
// Headless render harness, also exposed (read-only) to the workspace `bdd` integration target via `test-harness` (#805).
#[cfg(any(test, feature = "test-harness"))]
#[path = "tui_harness.rs"]
pub mod tui_harness;
#[cfg(test)]
#[path = "tui_harness_render_tests.rs"]
mod tui_harness_render_tests;
#[cfg(test)]
#[path = "tui_harness_tests.rs"]
mod tui_harness_tests;
