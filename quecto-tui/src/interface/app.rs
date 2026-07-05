use std::collections::BTreeMap;
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
use crate::interface::components::model_selector::{ModelSelector, ModelSelectorResult};
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

const MAX_ESCAPE_RETRIES: usize = 5;

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

pub struct App {
    terminal: Terminal,
    renderer: DiffRenderer<std::io::Stdout>,
    client: Client,
    editor: Editor,
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
    connected_agent_id: Option<String>,
    model_selector: Option<ModelSelector>,
    model_registry: app_models::ModelRegistry,
    resume_selector: Option<SelectList>,
    rewind: RewindState,
    subagents: SubagentUiState,
    render_log_path: Option<String>,
    #[cfg(any(test, feature = "test-harness"))]
    pub(super) rendered_frames: usize,
    #[cfg(any(test, feature = "test-harness"))]
    pub(super) suppress_paint: bool,
    selection: Option<TextSelection>,
    workflow_auto_continue: bool,
    workflow_completion_nudge: bool,
    git_branch: Option<String>,
    git_repo: Option<PathBuf>,
    last_rendered_lines: Vec<String>,
    context_stats_requested: bool,
    command_send_failure_tx: mpsc::Sender<CommandSendFailure>,
    command_send_failure_rx: mpsc::Receiver<CommandSendFailure>,
    started_at: tokio::time::Instant,
}

pub(crate) struct SessionView {
    chat: Chat,
    workflow_bar: workflow_bar::WorkflowBarState,
    running: bool,
    footer: Footer,
    deferred_subagent_notes: std::collections::VecDeque<String>,
    history_backfilled: bool,
    observed_run_state: bool,
}

impl SessionView {
    fn new(git_branch: Option<String>) -> Self {
        let mut footer = Footer::new();
        footer.set_git_branch(git_branch);
        Self::with_footer(footer)
    }

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Focus {
    Input,
    Panel,
}

const SUBAGENT_PANEL_WIDTH: usize = 30;

const MAX_RETAINED_SESSIONS: usize = 16;

struct CommandSendFailure {
    command_kind: &'static str,
    error: String,
}

#[derive(Default)]
pub(crate) struct RewindState {
    selector: Option<SelectList>,
    last_idle_escape: Option<tokio::time::Instant>,
    pending_open_id: Option<String>,
    pending_apply_id: Option<String>,
    request_seq: u64,
}

pub(crate) struct SubagentUiState {
    local: BTreeMap<String, TrackedSubagent>,
    frame: usize,
    awaited_agent_id: Option<String>,
    sessions: BTreeMap<String, SessionView>,
    session_order: Vec<String>,
    active_agent_id: Option<String>,
    panel_nav: crate::interface::components::list_navigator::ListNavigator,
    event_tx: mpsc::Sender<(String, Event)>,
    event_rx: mpsc::Receiver<(String, Event)>,
    active_conn: Option<(String, tokio::task::JoinHandle<()>)>,
    active_cmd_tx: Option<(String, mpsc::Sender<Command>)>,
    focus: Focus,
}

impl SubagentUiState {
    #[cfg(test)]
    pub(super) fn active_agent_id(&self) -> Option<&str> {
        self.active_agent_id.as_deref()
    }

    #[cfg(test)]
    pub(super) fn focus_region(&self) -> Focus {
        self.focus
    }

    fn new(
        event_tx: mpsc::Sender<(String, Event)>,
        event_rx: mpsc::Receiver<(String, Event)>,
    ) -> Self {
        Self {
            local: BTreeMap::new(),
            frame: 0,
            awaited_agent_id: None,
            sessions: BTreeMap::new(),
            session_order: Vec::new(),
            active_agent_id: None,
            panel_nav: crate::interface::components::list_navigator::ListNavigator::new(),
            event_tx,
            event_rx,
            active_conn: None,
            active_cmd_tx: None,
            focus: Focus::Input,
        }
    }
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
            model_registry: app_models::ModelRegistry::default(),
            resume_selector: None,
            rewind: RewindState::default(),
            subagents: SubagentUiState::new(subagent_event_tx, subagent_event_rx),
            render_log_path: std::env::var("QUECTO_TUI_RENDER_LOG").ok(),
            #[cfg(any(test, feature = "test-harness"))]
            rendered_frames: 0,
            #[cfg(any(test, feature = "test-harness"))]
            suppress_paint: false,
            selection: None,
            workflow_auto_continue: false,
            workflow_completion_nudge: false,
            git_branch,
            git_repo,
            last_rendered_lines: Vec::new(),
            context_stats_requested: false,
            command_send_failure_tx,
            command_send_failure_rx,
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

const MAX_CLIPBOARD_BYTES: usize = 100 * 1024;

fn copy_to_clipboard(text: &str) -> std::io::Result<()> {
    let mut stdout = std::io::stdout();
    write_osc52_clipboard_sequence(text, &mut stdout)
}

pub fn write_osc52_clipboard_sequence(text: &str, writer: &mut impl Write) -> std::io::Result<()> {
    let bytes = text.as_bytes();
    let capped = &bytes[..bytes.len().min(MAX_CLIPBOARD_BYTES)];
    let encoded = base64_encode(capped);
    let osc = format!("\x1b]52;c;{}\x07", encoded);
    writer.write_all(osc.as_bytes())?;
    writer.flush()
}

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

fn strip_ansi_for_selection(s: &str) -> String {
    crate::interface::ansi::strip_ansi(s)
}

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

#[derive(Debug)]
pub(crate) struct AgentRunState {
    running: bool,
    pending_aborts: u32,
    generation: u64,
}

const MAX_PENDING_ABORTS: u32 = 8;

impl AgentRunState {
    pub fn new() -> Self {
        Self {
            running: false,
            pending_aborts: 0,
            generation: 0,
        }
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn start(&mut self) {
        self.generation += 1;
        self.running = true;
        self.pending_aborts = 0;
    }

    pub fn abort(&mut self) {
        if self.running {
            if self.pending_aborts < MAX_PENDING_ABORTS {
                self.pending_aborts += 1;
            }
            self.running = false;
        }
    }

    pub fn end(&mut self) -> bool {
        if self.pending_aborts > 0 {
            self.pending_aborts -= 1;
            false
        } else {
            self.running = false;
            true
        }
    }

    pub fn reset(&mut self) {
        self.running = false;
        self.pending_aborts = 0;
    }
}

#[derive(Debug, PartialEq, Eq)]
enum CtrlCAction {
    ClearEditor,
    AbortAgent,
    Noop,
}

fn ctrl_c_action(agent_running: bool, editor_empty: bool) -> CtrlCAction {
    if !editor_empty {
        CtrlCAction::ClearEditor
    } else if agent_running {
        CtrlCAction::AbortAgent
    } else {
        CtrlCAction::Noop
    }
}

fn is_subagent_tool(tool_name: &str) -> bool {
    tool_name == "spawn" || tool_name == "agent_cmd"
}

const STATUS_EXITED: &str = "exited";

const EXITED_SUBAGENT_GRACE: Duration = Duration::from_secs(5);

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
#[cfg(any(test, feature = "test-harness"))]
#[path = "tui_harness.rs"]
pub mod tui_harness;
#[cfg(test)]
#[path = "tui_harness_render_tests.rs"]
mod tui_harness_render_tests;
#[cfg(test)]
#[path = "tui_harness_tests.rs"]
mod tui_harness_tests;
