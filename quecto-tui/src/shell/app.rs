use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

use crate::agents::focus::{Focus, MAX_RETAINED_SESSIONS, SUBAGENT_PANEL_WIDTH};
use crate::agents::view::FeedState;
use crate::agents::view::{SessionView, SubagentUi};
use crate::components::autocomplete::{Autocomplete, AutocompleteResult};
use crate::components::chat::{Chat, ChatEntry};
use crate::components::component::Component;
use crate::components::effort_selector::EffortSelectorResult;
use crate::components::footer::Footer;
use crate::components::kitty::KittyProtocol;
use crate::components::model_selector::ModelSelectorResult;
use crate::components::notification::{Notification, NotificationStack, NotifyLevel};
use crate::components::select_list::{SelectItem, SelectList};
use crate::components::selectable_item_modal::SelectableItemModal;
use crate::components::spinner::Spinner;
use crate::components::text_input::Editor;
use crate::components::workflow_bar;
use crate::protocol::client::{Client, Command, Event};
use crate::shell::keys::{self, Key};
use crate::shell::render::DiffRenderer;
use crate::shell::terminal::Terminal;
use crate::workspace::workspace_files::list_workspace_files;
use app_selection::TextSelection;
use tokio::sync::mpsc;

const SPINNER_TICK: Duration = Duration::from_millis(80);
pub(super) const STREAM_RENDER_INTERVAL: Duration = Duration::from_millis(33);
const MOUSE_SCROLL_LINES: usize = 3;
/// Raw unmarked paste is delimited by a quiet period. Unlike escape-key
/// disambiguation, each arriving chunk resets this deadline and there is no
/// read-count cap on the paste lifetime.
const RAW_PASTE_QUIET_TIMEOUT: Duration = Duration::from_millis(10);

#[path = "app_commands.rs"]
mod app_commands;
#[path = "tool_policy.rs"]
mod tool_policy;
#[cfg(any(test, feature = "test-harness"))]
#[path = "tui_harness_tool_policy.rs"]
mod tui_harness_tool_policy;
use app_commands::builtin_commands;
use app_message_recovery::{MessageRecoveryBatch, PendingMessageRecovery};

/// Application state.
pub struct App {
    terminal: Terminal,
    renderer: DiffRenderer<std::io::Stdout>,
    /// Per-tab connection states (#1465 / epic #1467). Indexed by [`TabId`];
    /// the active tab is selected by `active_tab`. Call sites reach the
    /// active slot via `ac()` / `ac()`, and a specific
    /// tab via `conn_for` / `conn_mut`.
    tabs: std::collections::HashMap<
        crate::shell::connection::TabId,
        connection_state::ConnectionState,
    >,
    /// Which tab is focused for input, render, and active command send.
    active_tab: crate::shell::connection::TabId,
    /// This TUI's workspace identity (#1466 decision 1): a UUID minted at
    /// startup (never cwd-derived) plus its auto-generated human label.
    pub(crate) workspace_id: String,
    pub(crate) workspace_label: String,
    /// When set, `active_conn(_mut)` temporarily targets this tab so inbound
    /// `SourcedEvent` routing can mutate the owner without flipping focus.
    routing_tab_override: Option<crate::shell::connection::TabId>,
    editor: Editor,
    autocomplete: Autocomplete,
    workspace: WorkspaceFlow,
    notifications: NotificationStack,
    kitty: KittyProtocol,
    should_exit: bool,
    stdin_buffer: crate::shell::stdin_buffer::StdinBuffer,
    /// Global selector-overlay half of the inference flow; per-tab
    /// model/effort state lives on `conn` (#1463).
    inference: InferenceFlow,
    /// Sub-agent / multi-session UI state (#997).
    subagents: SubagentUi,
    /// Diagnostic: with `QUECTO_TUI_RENDER_LOG` set, frames are appended here.
    render_log_path: Option<String>,
    #[cfg(any(test, feature = "test-harness"))]
    pub(super) rendered_frames: usize,
    /// Test-only: when set, [`App::render`] counts the frame but skips real
    /// stdout so headless tests don't garble the runner's terminal.
    #[cfg(any(test, feature = "test-harness"))]
    pub(super) suppress_paint: bool,
    /// Active mouse text selection (#528).
    selection: Option<TextSelection>,
    /// Last rendered lines (for extracting selected text from the buffer).
    last_rendered_lines: Vec<String>,
    /// Latest catalogue snapshot keyed by stable id (or name fallback) for future policy UI.
    tool_catalogue: HashMap<String, crate::protocol::client::ToolCatalogueEntry>,
    tool_policy_modal: Option<SelectableItemModal>,
    tool_policy_modal_pending_catalogue_id: Option<String>,
    command_send_failure_tx: mpsc::Sender<CommandSendFailure>,
    command_send_failure_rx: mpsc::Receiver<CommandSendFailure>,
    /// Completion channel for the OFF-LOOP disconnect diagnosis (#1462 scope
    /// 3): the `SourcedEvent::Closed` sentinel spawns the bounded child-exit /
    /// stderr-drain waits (#1047) onto a task carrying this sender, so a
    /// dying child can never stall the select loop; the loop finishes the
    /// disconnect when the diagnosis lands here.
    /// Keyed by the closing tab (#1470 r3) so N>1 tabs can never
    /// misattribute an exit detail.
    disconnect_diag_tx: mpsc::Sender<(crate::shell::connection::TabId, Option<String>)>,
    disconnect_diag_rx: mpsc::Receiver<(crate::shell::connection::TabId, Option<String>)>,
    /// Dedicated fan-in for master-connection events (`SourcedEvent::Tab` /
    /// `Closed`). A separate channel from the sub-agent fan-in restores the
    /// deleted dedicated select arm's fair interleave: master events and the
    /// close sentinel can no longer queue FIFO behind a chatty sub-agent
    /// burst (#1470 review). All tabs share this one channel, so the select
    /// arm count stays independent of N.
    pub(super) tab_event_rx: mpsc::Receiver<crate::shell::connection::SourcedEvent>,
    /// Sender half of `tab_event_rx`, retained so newly spawned/reattached tabs
    /// can join the same fan-in (#1465 AC1/AC2/AC6).
    pub(super) tab_event_tx: Option<mpsc::Sender<crate::shell::connection::SourcedEvent>>,
    /// Background tab spawn/reattach results (#1465).
    pub(super) tab_attach_tx: Option<mpsc::Sender<tab_lifecycle::TabAttachOutcome>>,
    pub(super) tab_attach_rx: mpsc::Receiver<tab_lifecycle::TabAttachOutcome>,
    /// Monotonic attach epoch so recycled TabIds reject stale spawn outcomes (#1465 F2).
    pub(super) next_attach_generation: u64,
    /// Parent CLI policy inherited by secondary tab spawns (#1465 F8).
    pub(crate) tab_spawn_policy: Option<crate::shell::cli::TabSpawnPolicy>,
}

/// Id of the TUI's single (master) agent connection. With one replicant
/// agent per tab (#1463, epic #1467) each connection carries its own id.
pub(crate) const MASTER_CONNECTION_ID: &str = "master";

struct CommandSendFailure {
    command: Command,
    error: String,
    /// Connection the send failed on — `MASTER_CONNECTION_ID` for today's
    /// single connection — so the rollback/notice cannot be misrouted
    /// cross-tab once there are N per-tab connections (#1460).
    connection: String,
}

impl App {
    pub fn new(terminal: Terminal, client: Client) -> Self {
        let mut footer = Footer::new();
        let git_repo = std::env::current_dir().ok();
        let git_branch = git_repo.as_deref().and_then(app_git::read_git_branch_from);
        footer.set_git_branch(git_branch.clone());
        let (command_send_failure_tx, command_send_failure_rx) = mpsc::channel(16);
        let (disconnect_diag_tx, disconnect_diag_rx) = mpsc::channel(4);

        // Sub-agent feeds fan into the channel on the sub-agent UI state;
        // master-connection events ride their own dedicated fan-in so they
        // interleave fairly with sub-agent bursts (#1462 / #1470 review).
        let subagents = SubagentUi::new();
        let (tab_event_tx, tab_event_rx) = mpsc::channel(256);
        let (tab_attach_tx, tab_attach_rx) = mpsc::channel(8);
        let connection = crate::shell::connection::Connection::spawn(
            client,
            crate::shell::connection::TabId::MASTER,
            tab_event_tx.clone(),
        );

        let thinking_visible = thinking_preferences::load_thinking_visible();

        let mut app = Self {
            terminal,
            renderer: DiffRenderer::new(std::io::stdout()),
            tabs: {
                let mut tabs = std::collections::HashMap::new();
                tabs.insert(
                    crate::shell::connection::TabId::MASTER,
                    connection_state::ConnectionState::new(
                        connection,
                        SessionView::with_footer(footer),
                    ),
                );
                tabs
            },
            active_tab: crate::shell::connection::TabId::MASTER,
            workspace_id: crate::shell::workspace_manifest::generate_workspace_id(),
            workspace_label: crate::shell::workspace_manifest::generate_workspace_label(),
            routing_tab_override: None,
            editor: Editor::new(),
            autocomplete: Autocomplete::new(builtin_commands().to_vec(), 8),
            workspace: WorkspaceFlow::new(git_branch, git_repo),
            notifications: NotificationStack::new(),
            kitty: KittyProtocol::new(),
            should_exit: false,
            stdin_buffer: crate::shell::stdin_buffer::StdinBuffer::new(),
            inference: InferenceFlow::default(),
            subagents,
            render_log_path: std::env::var("QUECTO_TUI_RENDER_LOG").ok(),
            #[cfg(any(test, feature = "test-harness"))]
            rendered_frames: 0,
            #[cfg(any(test, feature = "test-harness"))]
            suppress_paint: false,
            selection: None,
            last_rendered_lines: Vec::new(),
            tool_catalogue: HashMap::new(),
            tool_policy_modal: None,
            tool_policy_modal_pending_catalogue_id: None,
            command_send_failure_tx,
            command_send_failure_rx,
            disconnect_diag_tx,
            disconnect_diag_rx,
            tab_event_rx,
            tab_event_tx: Some(tab_event_tx),
            tab_attach_tx: Some(tab_attach_tx),
            tab_attach_rx,
            next_attach_generation: 1,
            tab_spawn_policy: None,
        };
        app.set_thinking_visibility(thinking_visible);
        app
    }

    pub(super) fn apply_git_branch(&mut self, branch: Option<String>) -> bool {
        if branch == self.workspace.git_branch {
            return false;
        }
        self.workspace.git_branch = branch.clone();
        self.ac_mut().master_session.footer.set_git_branch(branch);
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
        let Some(repo) = self.workspace.git_repo.clone() else {
            return;
        };
        *in_flight = true;
        let tx = tx.clone();
        tokio::task::spawn_blocking(move || {
            let branch = app_git::read_git_branch_from(&repo);
            let _ = tx.blocking_send(branch);
        });
    }

    fn files_autocomplete_root(&self) -> PathBuf {
        self.workspace
            .root
            .clone()
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."))
    }

    fn start_files_autocomplete_load(
        &self,
        tx: &mpsc::Sender<(PathBuf, Vec<String>)>,
        in_flight: &mut bool,
    ) {
        if *in_flight {
            return;
        }
        let root = self.files_autocomplete_root();
        *in_flight = true;
        let tx = tx.clone();
        tokio::task::spawn_blocking(move || {
            let files = list_workspace_files(&root);
            let _ = tx.blocking_send((root, files));
        });
    }

    fn apply_files_autocomplete_load(&mut self, root: PathBuf, files: Vec<String>) -> bool {
        if self.files_autocomplete_root() != root {
            return false;
        }
        self.workspace.files_autocomplete.apply_loaded_files(files);
        self.refresh_files_autocomplete_from_editor();
        true
    }

    fn refresh_files_autocomplete_from_editor(&mut self) {
        let line = self.editor.current_line().to_string();
        let col = self.editor.cursor_col();
        self.workspace.files_autocomplete.update(&line, col);
    }
}

#[path = "connection_state.rs"]
mod connection_state;

#[path = "app_disconnect.rs"]
mod app_disconnect;
// #1257: feature-owned controllers live under their capability modules;
// shell composes them as App extensions without taking ownership of policy.
#[path = "../inference/controller_effort.rs"]
mod app_effort;
#[path = "app_event_loop.rs"]
mod app_event_loop;
#[path = "app_events.rs"]
mod app_events;
#[path = "../workspace/controller_git.rs"]
mod app_git;
#[path = "../inference/controller_inference.rs"]
mod app_inference;
// Feature controllers own policy; shell composes their App extension methods.
#[path = "../conversation/controller_rewind_state.rs"]
mod app_rewind_state;
#[path = "../sessions/controller_sessions.rs"]
mod app_sessions;
#[path = "../workflow/controller_workflow.rs"]
mod app_workflow;
#[path = "../workspace/controller_workspace.rs"]
mod app_workspace;
use app_inference::InferenceFlow;
use app_rewind_state::RewindFlow;
use app_sessions::SessionsFlow;
use app_workflow::WorkflowFlow;
use app_workspace::WorkspaceFlow;
#[path = "app_stdin.rs"]
mod app_stdin;
pub const GIT_BRANCH_POLL_INTERVAL: std::time::Duration = app_git::GIT_BRANCH_POLL_INTERVAL;
#[path = "app_idle_efficiency.rs"]
mod app_idle_efficiency;
// #1257: agents-owned controllers live under `agents/`; shell composes them.
#[path = "../agents/controller_ledger_sync.rs"]
mod app_ledger_sync;
#[path = "../conversation/controller_message_recovery.rs"]
pub(crate) mod app_message_recovery;
#[path = "app_methods.rs"]
mod app_methods;
#[path = "app_methods_send.rs"]
mod app_methods_send;
#[path = "../inference/controller_models.rs"]
mod app_models;
#[path = "../conversation/controller_paged_history.rs"]
mod app_paged_history;
#[path = "app_render_helpers.rs"]
mod app_render_helpers;
#[path = "app_response.rs"]
mod app_response;
#[path = "app_response_set_model.rs"]
mod app_response_set_model;
#[cfg(any(test, feature = "test-harness"))]
#[path = "app_response_test_api.rs"]
mod app_response_test_api;
#[path = "../conversation/controller_resumed_history.rs"]
mod app_resumed_history;
#[path = "../conversation/controller_rewind.rs"]
mod app_rewind;
#[path = "app_selection.rs"]
mod app_selection;
#[path = "../agents/controller_subagent_environment.rs"]
mod app_subagent_environment;
#[path = "../agents/controller_subagent_feed.rs"]
mod app_subagent_feed;
#[path = "../agents/controller_subagent_panel.rs"]
mod app_subagent_panel;
#[path = "../agents/controller_subagent_panel_rows.rs"]
mod app_subagent_panel_rows;
#[path = "app_submit.rs"]
mod app_submit;
#[path = "app_thinking_visibility.rs"]
mod app_thinking_visibility;
#[path = "app_time.rs"]
mod app_time;
#[path = "tab_activity.rs"]
mod tab_activity;
#[path = "tab_lifecycle.rs"]
mod tab_lifecycle;
#[path = "thinking_preferences.rs"]
mod thinking_preferences;
#[path = "workspace_resume.rs"]
mod workspace_resume;
use crate::agents::roster::{
    gc_exited_subagents, next_exited_subagent_gc_deadline, subagent_status_is_active,
};
type TrackedSubagent =
    crate::agents::roster::TrackedSubagent<crate::protocol::client::SubagentInfoEvent>;
#[path = "../agents/controller_subagent_stream.rs"]
mod app_subagent_stream;
#[path = "../agents/controller_subagents.rs"]
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
    crate::components::ansi::strip_ansi(s)
}

/// Truncate tool arguments for spinner display.
fn truncate_args(args: &str) -> String {
    // sanitize_control (rather than a printable-range filter) so CSI bodies —
    // whose bytes are all printable — don't leak into the spinner line.
    crate::components::utils::sanitize_truncate_width_with_ellipsis(args, 40, "...")
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

const EXITED_SUBAGENT_GRACE: Duration = Duration::from_secs(5);

fn sanitize_workflow_status_text(text: &str, max_chars: usize) -> String {
    crate::components::utils::sanitize_truncate_width_with_ellipsis(text, max_chars, "…")
}

fn sanitize_agent_id(id: &str) -> String {
    crate::components::ansi::sanitize_control(id)
}

#[cfg(test)]
#[path = "../agents/app_agents_characterization_tests.rs"]
mod app_agents_characterization_tests;
#[cfg(test)]
#[path = "app_attach_backfill_tests.rs"]
mod app_attach_backfill_tests;
#[cfg(test)]
#[path = "app_bottom_spacing_tests.rs"]
mod app_bottom_spacing_tests;
#[cfg(test)]
#[path = "app_clipboard_tests.rs"]
mod app_clipboard_tests;
#[cfg(test)]
#[path = "app_conversation_characterization_tests/mod.rs"]
mod app_conversation_characterization_tests;
#[cfg(test)]
#[path = "app_cov_tests.rs"]
mod app_cov_tests;
#[cfg(test)]
#[path = "../agents/app_delete_all_subagents_tests.rs"]
mod app_delete_all_subagents_tests;
#[cfg(test)]
#[path = "app_disconnect_tests.rs"]
mod app_disconnect_tests;
#[cfg(test)]
#[path = "app_event_loop_abort_tests.rs"]
mod app_event_loop_abort_tests;
#[cfg(test)]
#[path = "app_event_loop_cov_tests.rs"]
mod app_event_loop_cov_tests;
#[cfg(test)]
#[path = "app_event_loop_tests.rs"]
mod app_event_loop_tests;
#[cfg(test)]
#[path = "app_events_1060_lifecycle_tests.rs"]
mod app_events_1060_lifecycle_tests;
#[cfg(test)]
#[path = "app_events_1060_tests.rs"]
mod app_events_1060_tests;
#[cfg(test)]
#[path = "app_fix_pass_1466_round2_tests.rs"]
mod app_fix_pass_1466_round2_tests;
#[cfg(test)]
#[path = "app_fix_pass_1466_tests.rs"]
mod app_fix_pass_1466_tests;
#[cfg(test)]
#[path = "app_fix_pass_1485_review_tests.rs"]
mod app_fix_pass_1485_review_tests;
#[cfg(test)]
#[path = "app_get_state_session_key_tests.rs"]
mod app_get_state_session_key_tests;
#[cfg(test)]
#[path = "../workspace/app_git_tests.rs"]
mod app_git_tests;
#[cfg(test)]
#[path = "app_idle_efficiency_tests.rs"]
mod app_idle_efficiency_tests;
#[cfg(test)]
#[path = "app_input_paste_tests.rs"]
mod app_input_paste_tests;
#[cfg(test)]
#[path = "app_issue1567_tests.rs"]
mod app_issue1567_tests;
#[cfg(test)]
#[path = "../agents/app_live_inflight_1259_tests.rs"]
mod app_live_inflight_1259_tests;
#[cfg(test)]
#[path = "app_methods_tests.rs"]
mod app_methods_tests;
#[cfg(test)]
#[path = "app_multi_tab_polish_tests.rs"]
mod app_multi_tab_polish_tests;
#[cfg(test)]
#[path = "../conversation/app_paged_history_review_tests.rs"]
mod app_paged_history_review_tests;
#[cfg(test)]
#[path = "../conversation/app_paged_history_tests.rs"]
mod app_paged_history_tests;
#[cfg(test)]
#[path = "app_refresh_tui_tests.rs"]
mod app_refresh_tui_tests;
#[cfg(test)]
#[path = "app_response_multiclient_tests.rs"]
mod app_response_multiclient_tests;
#[cfg(test)]
#[path = "../conversation/app_rewind_paged_load_tests.rs"]
mod app_rewind_paged_load_tests;
#[cfg(test)]
#[path = "../conversation/app_rewind_response_tests.rs"]
mod app_rewind_response_tests;
#[cfg(test)]
#[path = "app_selection_tests.rs"]
mod app_selection_tests;
#[cfg(test)]
#[path = "app_socket_path_tests.rs"]
mod app_socket_path_tests;
#[cfg(test)]
#[path = "app_sourced_event_tests.rs"]
mod app_sourced_event_tests;
#[cfg(test)]
#[path = "app_streaming_stability_tests.rs"]
mod app_streaming_stability_tests;
#[cfg(test)]
#[path = "../agents/app_subagent_compact_roster_tests.rs"]
mod app_subagent_compact_roster_tests;
#[cfg(test)]
#[path = "../agents/app_subagent_environment_body_tests.rs"]
mod app_subagent_environment_body_tests;
#[cfg(test)]
#[path = "../agents/app_subagent_environment_tests.rs"]
mod app_subagent_environment_tests;
#[cfg(test)]
#[path = "../agents/app_subagent_first_tests.rs"]
mod app_subagent_first_tests;
#[cfg(test)]
#[path = "../agents/app_subagent_panel_observer_tests.rs"]
mod app_subagent_panel_observer_tests;
#[cfg(test)]
#[path = "../agents/app_subagent_panel_scroll_tests.rs"]
mod app_subagent_panel_scroll_tests;
#[cfg(test)]
#[path = "../agents/app_subagent_panel_tests.rs"]
mod app_subagent_panel_tests;
#[cfg(test)]
#[path = "../agents/app_subagent_roster_authority_tests.rs"]
mod app_subagent_roster_authority_tests;
#[cfg(test)]
#[path = "../agents/app_subagent_scroll_1435_tests.rs"]
mod app_subagent_scroll_1435_tests;
#[cfg(test)]
#[path = "../agents/app_subagent_workflow_sticky_tests.rs"]
mod app_subagent_workflow_sticky_tests;
#[cfg(test)]
#[path = "../agents/app_subagents_tests.rs"]
mod app_subagents_tests;
#[cfg(test)]
#[path = "app_tab_collection_tests.rs"]
mod app_tab_collection_tests;
#[cfg(test)]
#[path = "../agents/app_tab_render_tests.rs"]
mod app_tab_render_tests;
#[cfg(test)]
#[path = "app_text_input_1277_tests.rs"]
mod app_text_input_1277_tests;
#[cfg(test)]
#[path = "app_tool_policy_cache_tests.rs"]
mod app_tool_policy_cache_tests;
#[cfg(test)]
#[path = "app_tool_policy_tests.rs"]
mod app_tool_policy_tests;
#[cfg(test)]
#[path = "../workflow/app_workflow_box_width_tests.rs"]
mod app_workflow_box_width_tests;
#[cfg(test)]
#[path = "../workflow/app_workflow_flow_tests.rs"]
mod app_workflow_flow_tests;
#[cfg(test)]
#[path = "app_chat_session_tests.rs"]
mod chat_session_tests;
#[cfg(test)]
#[path = "app_focus_parity_tests.rs"]
mod focus_parity_tests;
#[cfg(test)]
#[path = "../agents/app_subagent_selection_tests.rs"]
mod subagent_selection_tests;
#[cfg(test)]
#[path = "app_tests.rs"]
mod tests;
#[cfg(any(test, feature = "test-harness"))]
#[path = "tui_harness.rs"]
pub mod tui_harness;
#[cfg(test)]
#[path = "tui_harness_render_tests.rs"]
mod tui_harness_render_tests;
#[cfg(test)]
#[path = "tui_harness_tests.rs"]
mod tui_harness_tests;
