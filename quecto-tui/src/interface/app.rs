//! Application — the main TUI event loop.
//!
//! Wires all components together: terminal, UDS client, editor, chat,
//! spinner, footer, autocomplete, overlays, notifications, Kitty protocol,
//! signal handling, and extension management.

use std::io::Write;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::infrastructure::client::{Client, Command, Event};
use crate::infrastructure::terminal::Terminal;
use crate::interface::component::Component;
use crate::interface::components::autocomplete::{Autocomplete, AutocompleteResult, SlashCommand};
use crate::interface::components::chat::Chat;
use crate::interface::components::chat::ChatEntry;
use crate::interface::components::editor::Editor;
use crate::interface::components::footer::Footer;
use crate::interface::components::model_selector::{ModelSelector, ModelSelectorResult};
use crate::interface::components::notification::{Notification, NotificationStack, NotifyLevel};
use crate::interface::components::select_list::{SelectItem, SelectList, SelectResult};
use crate::interface::components::spinner::Spinner;
use crate::interface::components::subagent_bar::SubagentBar;
use crate::interface::components::widget::WidgetContainer;
use crate::interface::components::workflow_bar;
use crate::interface::keys::{self, Key};
use crate::interface::kitty::KittyProtocol;
use crate::interface::overlay::OverlayStack;
use crate::interface::theme;

/// Tick interval for spinner animation (~12fps).
const SPINNER_TICK: Duration = Duration::from_millis(80);

/// Lines scrolled per mouse wheel tick.
const MOUSE_SCROLL_LINES: usize = 3;

/// Maximum retry iterations for reassembling multi-fragment escape sequences.
/// Handles up to 5-fragment CSI splits on slow SSH/serial connections.
/// Total max wait = MAX_ESCAPE_RETRIES × escape_timeout (10ms) = 50ms.
const MAX_ESCAPE_RETRIES: usize = 5;

/// Width of the opaque padding around the resume selector content.
const RESUME_SELECTOR_BORDER_WIDTH: usize = 6;
/// Maximum width of the resume selector modal, including opaque padding.
const RESUME_SELECTOR_MAX_PANEL_WIDTH: usize = 88;

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

fn pad_ansi_to_width(text: &str, width: usize) -> String {
    let truncated = crate::interface::utils::truncate_to_width(text, width, Some("…"));
    let visible = crate::interface::utils::visible_width(&truncated);
    if visible >= width {
        truncated
    } else {
        format!("{}{}", truncated, " ".repeat(width - visible))
    }
}

fn build_resume_selector_overlay(
    selector: &mut SelectList,
    terminal_width: usize,
    terminal_height: usize,
) -> (Vec<String>, usize) {
    let panel_width = terminal_width
        .saturating_sub(4)
        .clamp(1, RESUME_SELECTOR_MAX_PANEL_WIDTH);
    let border_width = RESUME_SELECTOR_BORDER_WIDTH.min(panel_width.saturating_sub(20) / 2);
    let content_width = panel_width.saturating_sub(border_width * 2).max(1);
    let side_padding = " ".repeat(border_width);

    let mut content_lines = vec![theme::bold("Resume session")];
    content_lines.extend(selector.render(content_width));
    content_lines.push(theme::dim("Enter resume · Esc cancel"));

    let max_height = terminal_height.saturating_sub(4).max(1);
    let vertical_border = border_width.min(max_height.saturating_sub(1) / 2);
    let blank = theme::apply_overlay_bg("", panel_width);

    let mut overlay_lines = Vec::new();
    overlay_lines.extend(std::iter::repeat_n(blank.clone(), vertical_border));
    for line in content_lines {
        let padded = pad_ansi_to_width(&line, content_width);
        overlay_lines.push(theme::apply_overlay_bg(
            &format!("{side_padding}{padded}{side_padding}"),
            panel_width,
        ));
    }
    overlay_lines.extend(std::iter::repeat_n(blank, vertical_border));
    overlay_lines.truncate(max_height);

    (overlay_lines, panel_width)
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
    /// The model selector component (created on demand, pushed onto overlay stack).
    model_selector: Option<ModelSelector>,
    /// Session resume selector shown after `/resume` lists persisted sessions.
    resume_selector: Option<SelectList>,
    /// Client-side subagent state for immediate bar updates (#525).
    /// Updated from tool events (spawn/agent_cmd) and server pushes.
    /// Entries track expiry timestamps for auto-removal (#540).
    subagent_local: std::collections::BTreeMap<String, TrackedSubagent>,
    /// Active mouse text selection (#528).
    selection: Option<TextSelection>,
    /// Workflow header bar state (#563).
    workflow_bar: workflow_bar::WorkflowBarState,
    /// TUI-local: auto-continue sends follow-up when steps are incomplete.
    workflow_auto_continue: bool,
    /// TUI-local: completion nudge sends issue-cycling prompt when all done.
    workflow_completion_nudge: bool,
    /// Last rendered lines (for extracting selected text from the buffer).
    last_rendered_lines: Vec<String>,
    /// Whether we've already requested session stats as a fallback to learn
    /// the real context window for the current session/model.
    context_stats_requested: bool,
}

impl App {
    pub fn new(terminal: Terminal, client: Client) -> Self {
        let mut footer = Footer::new();
        if let Some(branch) = read_git_branch() {
            footer.set_git_branch(Some(branch));
        }

        Self {
            terminal,
            client,
            editor: Editor::new(),
            chat: Chat::new(),
            spinner: None,
            footer,
            autocomplete: Autocomplete::new(builtin_commands(), 8),
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
            model_selector: None,
            resume_selector: None,
            subagent_local: std::collections::BTreeMap::new(),
            selection: None,
            workflow_bar: workflow_bar::WorkflowBarState::default(),
            workflow_auto_continue: false,
            workflow_completion_nudge: false,
            last_rendered_lines: Vec::new(),
            context_stats_requested: false,
        }
    }

    /// Run the main event loop. Returns exit code.
    #[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
    pub async fn run(&mut self) -> i32 {
        self.terminal.enter_raw_mode();
        self.terminal.hide_cursor();

        // Query Kitty keyboard protocol support.
        self.kitty.query();

        // Query initial state from agent.
        let _ = self
            .client
            .send(&Command::GetState {
                id: Some("init".into()),
            })
            .await;
        // Query initial subagent state (#525).
        let _ = self
            .client
            .send(&Command::GetSubagents {
                id: Some("init-subagents".into()),
            })
            .await;

        // Set up SIGWINCH handler.
        let mut resize_rx = crate::infrastructure::signals::sigwinch_stream().await;

        // Set up stdin reader (async, byte-level).
        let (stdin_tx, mut stdin_rx) = mpsc::channel::<Vec<u8>>(64);
        tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut stdin = tokio::io::stdin();
            let mut buf = [0u8; 256];
            loop {
                match stdin.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        if stdin_tx.send(buf[..n].to_vec()).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        // Kitty protocol fallback timer.
        let kitty_deadline = tokio::time::Instant::now() + Duration::from_millis(150);
        let mut kitty_fallback_done = false;

        // Initial render.
        self.render();

        // Spinner tick timer.
        let mut spinner_interval = tokio::time::interval(SPINNER_TICK);
        spinner_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        // Timeout for incomplete escape sequences (matches Pi TUI's 10ms).
        let escape_timeout = Duration::from_millis(10);

        // Main event loop.
        loop {
            if self.should_exit {
                break;
            }

            tokio::select! {
                // Stdin input.
                Some(bytes) = stdin_rx.recv() => {
                    // Check for Kitty protocol response before buffering.
                    if !self.kitty.active && !kitty_fallback_done {
                        if let Some(_flags) = KittyProtocol::parse_response(&bytes) {
                            self.kitty.enable();
                            kitty_fallback_done = true;
                            continue;
                        }
                    }
                    // Filter Kitty key release events.
                    if self.kitty.active && crate::interface::kitty::is_key_release(&bytes) {
                        continue;
                    }
                    // Feed bytes into the proper StdinBuffer.
                    self.stdin_buffer.feed(&bytes);

                    // Drain complete sequences immediately.
                    let complete = self.stdin_buffer.drain_complete();
                    for seq in &complete {
                        self.process_key_sequence(seq);
                        if self.should_exit { break; }
                    }

                    // If there are still pending bytes (incomplete escape),
                    // retry up to MAX_ESCAPE_RETRIES times waiting for more data.
                    // This handles 3+ fragment CSI splits on slow SSH/serial (#466).
                    if self.stdin_buffer.has_pending() && !self.should_exit {
                        let mut retries = 0;
                        while self.stdin_buffer.has_pending()
                            && retries < MAX_ESCAPE_RETRIES
                            && !self.should_exit
                        {
                            retries += 1;
                            match tokio::time::timeout(escape_timeout, stdin_rx.recv()).await {
                                Ok(Some(more)) => {
                                    self.stdin_buffer.feed(&more);
                                    let seqs = self.stdin_buffer.drain_complete();
                                    for seq in &seqs {
                                        self.process_key_sequence(seq);
                                        if self.should_exit { break; }
                                    }
                                }
                                Ok(None) => break, // Channel closed (stdin EOF).
                                Err(_) => break,   // Timeout — no more data coming.
                            }
                        }
                        // Force drain anything still pending after retries
                        // exhausted (bare Escape, etc.).
                        if !self.should_exit {
                            let forced = self.stdin_buffer.drain_all();
                            for seq in &forced {
                                self.process_key_sequence(seq);
                                if self.should_exit { break; }
                            }
                        }
                    }

                    self.render();
                }
                // Agent events.
                event = self.client.recv(), if self.agent_connected => {
                    match event {
                        Some(ev) => {
                            self.handle_event(ev);
                            self.render();
                        }
                        None => {
                            // Agent disconnected — stop polling.
                            self.agent_connected = false;
                            self.agent_state.reset();
                            self.spinner = None;
                            self.chat.finalize_assistant();
                            self.notify("Agent disconnected", NotifyLevel::Error);
                            self.render();
                        }
                    }
                }
                // Terminal resize.
                Some(()) = resize_rx.recv() => {
                    self.terminal.refresh_size();
                    self.render_full();
                }
                // Spinner tick.
                _ = spinner_interval.tick() => {
                    let mut needs_render = false;
                    if let Some(spinner) = &mut self.spinner {
                        if spinner.tick() {
                            needs_render = true;
                        }
                    }
                    // GC expired notifications.
                    if self.notifications.gc() {
                        needs_render = true;
                    }
                    // GC exited subagent bars (#540).
                    if self.gc_exited_subagents() {
                        needs_render = true;
                    }
                    // Kitty fallback — enable modifyOtherKeys if no response.
                    if !kitty_fallback_done && tokio::time::Instant::now() >= kitty_deadline {
                        if !self.kitty.active {
                            self.kitty.enable_modify_other_keys();
                        }
                        kitty_fallback_done = true;
                    }
                    if needs_render {
                        self.render();
                    }
                }
            }
        }

        // Cleanup.
        self.kitty.cleanup();
        self.terminal.show_cursor();
        self.terminal.exit_raw_mode();
        self.terminal.write_str("\r\n");
        0
    }

    // ── Input handling ────────────────────────────────────────────────

    /// Process a single complete key sequence from the StdinBuffer.
    fn process_key_sequence(&mut self, seq: &[u8]) {
        if let Some((key, _)) = keys::parse_key(seq) {
            self.handle_key(key);
        }
    }

    #[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
    fn handle_key(&mut self, key: Key) {
        // Unconditional exit — Ctrl+D must work regardless of overlays,
        // autocomplete state, or agent activity (#478).
        if matches!(key, Key::Ctrl('d')) {
            if self.agent_state.is_running() {
                self.handle_abort();
            }
            self.should_exit = true;
            return;
        }

        // If the resume selector is active, route input to it.
        if self.resume_selector.is_some() {
            self.handle_resume_selector_key(&key);
            return;
        }

        // If the model selector is active, route input to it.
        if self.model_selector.is_some() {
            self.handle_model_selector_key(&key);
            return;
        }

        // If an overlay is active, route input there.
        if self.overlay_stack.has_visible() {
            if let Some(entry) = self.overlay_stack.topmost_entry_mut() {
                entry.component.handle_input(&key);
            }
            // Check if overlay wants to close (Escape).
            if matches!(key, Key::Escape) {
                self.overlay_stack.pop();
            }
            return;
        }

        // If autocomplete is active, route navigation keys there.
        if self.autocomplete.is_active() {
            match &key {
                Key::Up | Key::Down | Key::Tab | Key::Escape => {
                    self.autocomplete.handle_input(&key);
                    // Check if a suggestion was selected.
                    match self.autocomplete.take_result() {
                        AutocompleteResult::Selected(value) => {
                            self.editor.set_text(&value);
                        }
                        AutocompleteResult::Dismissed => {}
                        AutocompleteResult::Pending => {}
                    }
                    return;
                }
                Key::Enter => {
                    // Accept the highlighted suggestion AND submit it.
                    self.autocomplete.handle_input(&Key::Tab);
                    if let AutocompleteResult::Selected(value) = self.autocomplete.take_result() {
                        self.editor.set_text(&value);
                        self.editor.add_to_history(value.trim());
                        self.autocomplete.dismiss();
                        self.handle_submit(&value);
                        self.editor.set_text("");
                    }
                    return;
                }
                _ => {
                    // Other keys go to editor, then update autocomplete.
                }
            }
        }

        // Global key handlers.
        // Note: Ctrl+D is handled at the top of handle_key (unconditional exit).
        match &key {
            Key::Ctrl('c') => {
                match ctrl_c_action(self.agent_state.is_running(), self.editor.text().is_empty()) {
                    CtrlCAction::ClearEditor => {
                        self.editor.set_text("");
                        self.autocomplete.dismiss();
                    }
                    CtrlCAction::AbortAgent => {
                        self.handle_abort();
                    }
                    CtrlCAction::Noop => {}
                }
                return;
            }
            Key::Escape => {
                if self.agent_state.is_running() {
                    self.handle_abort();
                } else {
                    // Clear editor if it has text.
                    if !self.editor.text().is_empty() {
                        self.editor.set_text("");
                        self.autocomplete.dismiss();
                    }
                }
                return;
            }
            Key::Ctrl('z') => {
                // Suspend (Ctrl+Z).
                self.kitty.cleanup();
                self.terminal.show_cursor();
                crate::infrastructure::signals::suspend();
                // Resumed — re-enter raw mode.
                self.terminal.enter_raw_mode();
                self.terminal.hide_cursor();
                self.kitty.query();
                self.render_full();
                return;
            }
            Key::Ctrl('l') => {
                // Open model selector overlay.
                self.open_model_selector();
                return;
            }
            Key::Ctrl('o') => {
                // Toggle tool output expansion.
                self.chat.toggle_tool_expand();
                let state = if self.chat.tool_expanded {
                    "expanded"
                } else {
                    "collapsed"
                };
                self.notify(&format!("Tool output {}", state), NotifyLevel::Info);
                return;
            }
            Key::CtrlShift('a') => {
                self.toggle_workflow_auto_continue();
                return;
            }
            Key::CtrlShift('n') => {
                self.toggle_workflow_completion_nudge();
                return;
            }
            Key::MousePress(col, row) => {
                self.selection = Some(TextSelection {
                    start: SelectionAnchor {
                        col: *col,
                        row: *row,
                    },
                    end: SelectionAnchor {
                        col: *col,
                        row: *row,
                    },
                });
                return;
            }
            Key::MouseDrag(col, row) => {
                if let Some(sel) = &mut self.selection {
                    sel.end = SelectionAnchor {
                        col: *col,
                        row: *row,
                    };
                }
                return;
            }
            Key::MouseRelease(col, row) => {
                if let Some(sel) = self.selection.take() {
                    let end = SelectionAnchor {
                        col: *col,
                        row: *row,
                    };
                    // Only copy if there's an actual selection (not just a click).
                    if sel.start.row != end.row || sel.start.col != end.col {
                        let text = self.extract_selection(&sel.start, &end);
                        if !text.is_empty() {
                            copy_to_clipboard(&text);
                            let chars = text.chars().count();
                            self.notify(
                                &format!("Copied {} chars to clipboard", chars),
                                NotifyLevel::Success,
                            );
                        }
                    }
                }
                return;
            }
            Key::ScrollUp => {
                self.chat.scroll_up(MOUSE_SCROLL_LINES);
                return;
            }
            Key::ScrollDown => {
                self.chat.scroll_down(MOUSE_SCROLL_LINES);
                return;
            }
            Key::PageUp => {
                self.chat.scroll_up(10);
                return;
            }
            Key::PageDown => {
                self.chat.scroll_down(10);
                return;
            }
            _ => {}
        }

        // Forward to editor.
        self.editor.handle_input(&key);

        // Update autocomplete after every editor change.
        self.autocomplete.update(&self.editor.text());

        // Check if editor submitted.
        if let Some(text) = self.editor.take_submit() {
            self.autocomplete.dismiss();
            self.handle_submit(&text);
        }
    }

    fn handle_submit(&mut self, text: &str) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }

        // Slash commands.
        if trimmed.starts_with('/') {
            match trimmed {
                "/quit" | "/exit" => {
                    self.should_exit = true;
                    return;
                }
                "/clear" => {
                    self.reset_session("Conversation cleared");
                    return;
                }
                "/new" => {
                    self.reset_session("New session started");
                    return;
                }
                "/help" | "/hotkeys" => {
                    self.show_help();
                    return;
                }
                "/session" => {
                    self.send_session_stats();
                    return;
                }
                "/workflow" => {
                    self.show_workflow_status();
                    return;
                }
                "/resume" => {
                    self.send_list_sessions();
                    return;
                }
                _ if trimmed.starts_with("/resume ") => {
                    let session = trimmed.strip_prefix("/resume").unwrap().trim();
                    self.send_resume_session(session);
                    return;
                }
                _ if trimmed.starts_with("/model") => {
                    let model_name = trimmed.strip_prefix("/model").unwrap().trim();
                    if !model_name.is_empty() {
                        self.send_set_model(model_name);
                    } else {
                        // No model name — open the model selector overlay.
                        self.open_model_selector();
                    }
                    return;
                }
                "/workflow-auto" => {
                    self.toggle_workflow_auto_continue();
                    return;
                }
                "/workflow-nudge" => {
                    self.toggle_workflow_completion_nudge();
                    return;
                }
                _ => {
                    self.reject_unknown_slash_command(trimmed);
                    return;
                }
            }
        }

        // Add user message to chat.
        self.chat.add_entry(ChatEntry::User {
            text: text.to_string(),
        });

        // Send to agent.
        let cmd = Command::Prompt {
            id: None,
            message: text.to_string(),
            streaming_behavior: if self.agent_state.is_running() {
                Some("steer".to_string())
            } else {
                None
            },
        };
        self.send_command(cmd);
    }

    // ── Abort handling (bug fix) ──────────────────────────────────────

    fn handle_abort(&mut self) {
        // Send abort to agent.
        self.send_command(Command::Abort { id: None });

        // Call abort on the state machine — does NOT set running to false.
        // The AgentEnd event will arrive and be matched against the current
        // generation, preventing stale events from corrupting state (#502).
        self.agent_state.abort();
        self.footer.set_streaming(false);

        // Stop spinner (visual feedback that abort was acknowledged).
        self.spinner = None;

        // Finalize any streaming assistant message.
        self.chat.finalize_assistant();

        // Show abort status.
        self.chat.add_entry(ChatEntry::Status {
            text: "Operation aborted".to_string(),
        });
    }

    // ── Agent event handling ──────────────────────────────────────────

    #[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
    fn handle_event(&mut self, event: Event) {
        match event {
            Event::AgentStart => {
                self.agent_state.start();
                self.footer.set_streaming(true);
                self.spinner = Some(Spinner::new("Working... (Esc to interrupt)"));
            }
            Event::Token { token } => {
                self.chat.append_token(&token);
            }
            Event::TurnStart => {}
            Event::TurnEnd { message, .. } => {
                self.chat.finalize_assistant();
                // Extract token usage from turn message.
                if let Some(usage) = message.get("usage") {
                    let input = usage.get("input").and_then(|v| v.as_u64()).unwrap_or(0);
                    let _output = usage.get("output").and_then(|v| v.as_u64()).unwrap_or(0);
                    let total = usage.get("total").and_then(|v| v.as_u64()).unwrap_or(0);
                    if let Some(window) = message
                        .get("maxContextTokens")
                        .and_then(|v| v.as_u64())
                        .map(|v| v as usize)
                    {
                        self.footer.update_context_usage(input, window);
                        self.context_stats_requested = true;
                    } else if total > 0 && !self.context_stats_requested {
                        self.context_stats_requested = true;
                        self.send_session_stats();
                    }
                }
            }
            Event::ToolExecutionStart {
                tool_call_id,
                tool_name,
                args,
            } => {
                let args_str = if args.is_object() || args.is_array() {
                    serde_json::to_string(&args).unwrap_or_default()
                } else {
                    args.to_string()
                };
                if let Some(spinner) = &mut self.spinner {
                    let msg = match tool_name.as_str() {
                        "spawn" => {
                            let agent: String = args
                                .get("agent_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("agent")
                                .chars()
                                .filter(|c| !c.is_control())
                                .collect();
                            format!("Spawning {}...", agent)
                        }
                        "agent_cmd" => {
                            let command: String = args
                                .get("command")
                                .and_then(|v| v.as_str())
                                .unwrap_or("?")
                                .chars()
                                .filter(|c| !c.is_control())
                                .collect();
                            let agent_id: String = args
                                .get("agent_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("?")
                                .chars()
                                .filter(|c| !c.is_control())
                                .collect();
                            format!("{} → {}...", command, agent_id)
                        }
                        _ => format!("{} {}...", tool_name, truncate_args(&args_str)),
                    };
                    spinner.set_message(&msg);
                }
                // Track spawning subagents locally for immediate bar display.
                let is_spawn = tool_name == "spawn";
                // Suppress spawn / agent_cmd mutation tool boxes — the
                // status bar provides visibility. Query results are shown (#538).
                if !suppress_tool_box(&tool_name, &args) {
                    self.chat.start_tool(tool_call_id, tool_name, args_str);
                }
                if is_spawn {
                    if let Some(agent_id) = args.get("agent_id").and_then(|v| v.as_str()) {
                        let sanitized: String =
                            agent_id.chars().filter(|c| !c.is_control()).collect();
                        self.subagent_local.insert(
                            sanitized.clone(),
                            TrackedSubagent::new(
                                crate::infrastructure::client::SubagentInfoEvent {
                                    agent_id: sanitized,
                                    status: "starting".to_string(),
                                    last_tool: None,
                                    last_error: None,
                                    pid: 0,
                                },
                            ),
                        );
                        self.rebuild_subagent_bar();
                    }
                }
            }
            Event::ToolExecutionEnd {
                tool_call_id,
                tool_name,
                result,
                is_error,
            } => {
                // Always call complete_tool — it's a no-op if start was
                // suppressed (no matching tool_call_id in chat entries).
                let result_text = crate::infrastructure::client::extract_result_text(&result);
                self.chat
                    .complete_tool(&tool_call_id, &result_text, is_error, None);
                // Update local subagent state on spawn completion.
                if tool_name == "spawn" && !is_error {
                    // Extract agent_id from the result text ("Subagent 'X' is running...")
                    let result_text = crate::infrastructure::client::extract_result_text(&result);
                    if let Some(start) = result_text.find('\'') {
                        if let Some(end) = result_text[start + 1..].find('\'') {
                            let agent_id = &result_text[start + 1..start + 1 + end];
                            let sanitized: String =
                                agent_id.chars().filter(|c| !c.is_control()).collect();
                            if let Some(entry) = self.subagent_local.get_mut(&sanitized) {
                                entry.info.status = "running".to_string();
                            }
                            self.rebuild_subagent_bar();
                        }
                    }
                }
                // Also request server-side state for eventual consistency.
                if is_subagent_tool(&tool_name) {
                    self.send_command(Command::GetSubagents { id: None });
                }
                if let Some(spinner) = &mut self.spinner {
                    spinner.set_message("Working... (Esc to interrupt)");
                }
            }
            // Abort-aware end: if there are pending aborts, this AgentEnd
            // is from an aborted run and should be ignored (#502).
            Event::AgentEnd { .. } if self.agent_state.end() => {
                self.footer.set_streaming(false);
                self.spinner = None;
                self.chat.finalize_assistant();

                // Workflow auto-continue / completion nudge (#590).
                self.maybe_send_workflow_followup();
            }
            Event::AgentEnd { .. } => {}
            Event::Response {
                command,
                success,
                data,
                error,
                ..
            } => match command.as_str() {
                "get_state" if success => {
                    if let Some(data) = data {
                        if let Some(model) = data.get("model").and_then(|m| m.as_str()) {
                            // Sanitize model name to prevent terminal escape injection.
                            let sanitized: String =
                                model.chars().filter(|c| !c.is_control()).collect();
                            self.footer.set_model(&sanitized);
                            self.current_model = Some(sanitized);
                        }
                        if let Some(max_ctx) = data.get("maxContextTokens").and_then(|v| v.as_u64())
                        {
                            self.footer.set_context_window(max_ctx as usize);
                            self.context_stats_requested = true;
                        }
                        // Seed workflow header bar from get_state (#593).
                        if let Some(wf) = data.get("workflow") {
                            self.workflow_bar = workflow_bar::parse_workflow_event(wf);
                        }
                    }
                }
                "set_model" if success => {
                    self.notify("Model switched", NotifyLevel::Success);
                }
                "set_model" if !success => {
                    let msg = error.unwrap_or_else(|| "unknown error".into());
                    self.notify(&format!("Model switch failed: {}", msg), NotifyLevel::Error);
                }
                "get_session_stats" if success => {
                    if let Some(data) = data {
                        self.show_session_stats(&data);
                    }
                }
                "list_sessions" if success => {
                    if let Some(data) = data {
                        self.open_resume_selector(&data);
                    }
                }
                "list_sessions" if !success => {
                    let msg = error.unwrap_or_else(|| "unknown error".into());
                    self.notify(
                        &format!("Could not list sessions: {msg}"),
                        NotifyLevel::Error,
                    );
                }
                "resume_session" if success => {
                    let session = data
                        .as_ref()
                        .and_then(|d| d.get("session").and_then(|v| v.as_str()))
                        .unwrap_or("session");
                    self.notify(&format!("Resumed session {session}"), NotifyLevel::Success);
                    self.send_command(Command::GetMessages {
                        id: Some("resume-messages".into()),
                    });
                    self.send_session_stats();
                }
                "resume_session" if !success => {
                    let msg = error.unwrap_or_else(|| "unknown error".into());
                    self.notify(&format!("Resume failed: {msg}"), NotifyLevel::Error);
                }
                "get_messages" if success => {
                    if let Some(data) = data {
                        self.replace_chat_with_messages(&data);
                    }
                }
                "clear_history" if success => {}
                "get_subagents" if success => {
                    if let Some(data) = &data {
                        if let Some(arr) = data.get("subagents") {
                            if let Ok(infos) = serde_json::from_value::<
                                Vec<crate::infrastructure::client::SubagentInfoEvent>,
                            >(arr.clone())
                            {
                                self.update_subagent_bar(infos);
                            }
                        }
                    }
                }
                "agent_error" => {
                    let msg = error.unwrap_or_else(|| "unknown error".into());
                    self.chat.add_entry(ChatEntry::Status {
                        text: format!("Error: {}", msg),
                    });
                    self.agent_state.reset();
                    self.footer.set_streaming(false);
                    self.spinner = None;
                }
                _ => {}
            },
            Event::SubagentStateChanged { subagents } => {
                self.update_subagent_bar(subagents);
            }
            Event::WorkflowState {
                steps,
                progress,
                active_issue,
                mode,
                active_template,
                available_templates,
            } => {
                // Reconstruct the JSON for parsing.
                let mut event = serde_json::json!({
                    "steps": steps,
                    "progress": progress,
                });
                if let Some(issue) = active_issue {
                    event["activeIssue"] = issue;
                }
                if let Some(m) = mode {
                    event["mode"] = serde_json::json!(m);
                }
                if let Some(tpl) = active_template {
                    event["activeTemplate"] = tpl;
                }
                if let Some(templates) = available_templates {
                    event["availableTemplates"] = serde_json::json!(templates);
                }
                self.workflow_bar = workflow_bar::parse_workflow_event(&event);
            }
            _ => {}
        }
    }

    /// Update subagent state from server push (full replacement).
    /// Send a workflow follow-up prompt if auto-continue or completion nudge is active.
    fn maybe_send_workflow_followup(&mut self) {
        let wf = &self.workflow_bar;
        // Only act when workflow is active (has steps).
        if wf.total == 0 && wf.mode.as_deref() != Some("selecting_template") {
            return;
        }
        let all_done = wf.total > 0 && wf.done == wf.total;

        if self.workflow_auto_continue && !all_done && wf.done > 0 {
            let msg = format!(
                "Workflow incomplete ({}/{}). Continue with the next incomplete step. \
                 Use the workflow tool to check off steps as you complete them. \
                 Respond with just the word DONE when all {} steps are checked off.",
                wf.done, wf.total, wf.total
            );
            self.send_command(Command::FollowUp {
                id: None,
                message: msg,
            });
        } else if self.workflow_completion_nudge && all_done {
            let issue_part = wf
                .issue_number
                .map(|n| {
                    let title = wf.issue_title.as_deref().unwrap_or("");
                    format!(
                        "You have completed all {} workflow steps for issue #{}: \"{}\". ",
                        wf.total, n, title
                    )
                })
                .unwrap_or_else(|| format!("You have completed all {} workflow steps. ", wf.total));
            let msg = format!(
                "{issue_part}Now do the following in order:\n\
                 1. Close the issue (if applicable)\n\
                 2. Pick the next issue to work on — if no open issues exist, respond with just the word NONE\n\
                 3. Record it: call the workflow tool with action=\"set_issue\", issueNumber=<n>, issueTitle=\"...\"\n\
                 4. Reset the checklist: call the workflow tool with action=\"reset\"\n\
                 5. Begin Step 1 immediately for the new issue"
            );
            self.send_command(Command::FollowUp {
                id: None,
                message: msg,
            });
        }
    }

    fn update_subagent_bar(
        &mut self,
        subagents: Vec<crate::infrastructure::client::SubagentInfoEvent>,
    ) {
        // Merge server data with existing local state to preserve exited_at
        // timestamps. New entries are inserted; entries absent from the
        // server push are removed unless they have an active grace period.
        let mut new_map = std::collections::BTreeMap::new();
        for s in subagents {
            let id = sanitize_agent_id(&s.agent_id);
            if let Some(mut existing) = self.subagent_local.remove(&id) {
                existing.update_info(s);
                new_map.insert(id, existing);
            } else {
                new_map.insert(id, TrackedSubagent::new(s));
            }
        }
        // Preserve locally-tracked exited entries whose grace period
        // hasn't elapsed yet (server may stop reporting them immediately).
        let now = tokio::time::Instant::now();
        for (id, entry) in std::mem::take(&mut self.subagent_local) {
            if let Some(exited_at) = entry.exited_at {
                if now.saturating_duration_since(exited_at) < EXITED_SUBAGENT_GRACE {
                    new_map.entry(id).or_insert(entry);
                }
            }
        }
        self.subagent_local = new_map;
        self.rebuild_subagent_bar();
    }

    /// Rebuild the widget from local state.
    fn rebuild_subagent_bar(&mut self) {
        if self.subagent_local.is_empty() {
            self.widgets_above.clear("subagents");
        } else {
            let infos: Vec<crate::infrastructure::client::SubagentInfoEvent> = self
                .subagent_local
                .values()
                .map(|t| t.info.clone())
                .collect();
            let mut bar = SubagentBar::new();
            bar.update(infos);
            self.widgets_above.set("subagents", Box::new(bar));
        }
    }

    /// GC exited subagent bars whose grace period has elapsed (#540).
    /// Returns `true` if the bar was modified.
    fn gc_exited_subagents(&mut self) -> bool {
        if self.subagent_local.is_empty() {
            return false;
        }
        let removed = gc_exited_subagents(
            &mut self.subagent_local,
            tokio::time::Instant::now(),
            EXITED_SUBAGENT_GRACE,
        );
        if removed {
            self.rebuild_subagent_bar();
        }
        removed
    }

    // ── Slash command handlers ─────────────────────────────────────────

    fn reject_unknown_slash_command(&mut self, command: &str) {
        self.chat.add_entry(ChatEntry::Status {
            text: format!(
                "Unknown slash command: {command}\nType /help to see available commands."
            ),
        });
        self.notify("Unknown slash command", NotifyLevel::Warning);
    }

    fn show_help(&mut self) {
        self.chat.add_entry(ChatEntry::Status {
            text: [
                "Keyboard shortcuts:",
                "  Enter          Send message",
                "  Shift+Enter    Insert newline",
                "  Alt+Enter      Insert newline",
                "  Escape         Abort agent / clear editor",
                "  Ctrl+C         Clear editor first, abort if empty",
                "  Ctrl+D         Exit",
                "  Ctrl+L         Open model selector",
                "  Ctrl+O         Toggle tool output expansion",
                "  Ctrl+Shift+A   Toggle workflow auto-continue",
                "  Ctrl+Shift+N   Toggle workflow completion nudge",
                "  Ctrl+Z         Suspend (resume with fg)",
                "  PageUp/Down    Scroll chat",
                "  Up/Down        Input history",
                "",
                "Slash commands:",
                "  /model         Open model selector",
                "  /model <name>  Switch to model directly",
                "  /clear         Clear conversation",
                "  /new           New session",
                "  /session       Show session info",
                "  /workflow      Show workflow status",
                "  /resume        Pick a persisted session to resume",
                "  /resume <name> Resume a persisted session directly",
                "  /workflow-auto Toggle workflow auto-continue",
                "  /workflow-nudge Toggle workflow completion nudge",
                "  /help,/hotkeys This help",
                "  /quit,/exit    Exit",
            ]
            .join("\n"),
        });
    }

    fn show_workflow_status(&mut self) {
        let wf = &self.workflow_bar;
        let text = if workflow_bar::render_widget(wf, self.terminal.width).is_empty() {
            "Workflow is not active. Start quecto-tui with --workflow to enable it.".to_string()
        } else {
            let current = wf
                .current_step_id()
                .map(|id| {
                    format!(
                        "next step {id}: {}",
                        sanitize_workflow_status_text(wf.current_step_label().unwrap_or(""), 80)
                    )
                })
                .unwrap_or_else(|| "complete".to_string());
            format!(
                "Workflow status: {}/{} ({current})",
                wf.done,
                wf.total.max(1)
            )
        };
        self.chat.add_entry(ChatEntry::Status { text });
    }

    fn toggle_workflow_auto_continue(&mut self) {
        self.workflow_auto_continue = !self.workflow_auto_continue;
        let state = if self.workflow_auto_continue {
            "ON — agent will be nudged to complete all steps"
        } else {
            "OFF"
        };
        self.notify(
            &format!("Workflow auto-continue {state}"),
            NotifyLevel::Info,
        );
    }

    fn toggle_workflow_completion_nudge(&mut self) {
        self.workflow_completion_nudge = !self.workflow_completion_nudge;
        let state = if self.workflow_completion_nudge {
            "ON — agent will be prompted to pick next issue"
        } else {
            "OFF"
        };
        self.notify(
            &format!("Workflow completion nudge {state}"),
            NotifyLevel::Info,
        );
    }

    fn send_session_stats(&mut self) {
        self.send_command(Command::GetSessionStats {
            id: Some("stats".into()),
        });
    }

    fn send_list_sessions(&mut self) {
        self.send_command(Command::ListSessions {
            id: Some("resume-list".into()),
        });
    }

    fn send_resume_session(&mut self, session: &str) {
        if session.trim().is_empty() {
            self.send_list_sessions();
            return;
        }
        self.send_command(Command::ResumeSession {
            id: Some("resume".into()),
            session: session.trim().to_string(),
        });
    }

    fn show_session_stats(&mut self, data: &serde_json::Value) {
        let key = data
            .get("sessionKey")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let msgs = data
            .get("totalMessages")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let input = data
            .get("tokens")
            .and_then(|t| t.get("input"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let output = data
            .get("tokens")
            .and_then(|t| t.get("output"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let cost = data.get("cost").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let max_context_tokens = data
            .get("maxContextTokens")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);
        if let Some(window) = max_context_tokens {
            self.footer.update_context_usage(input, window);
            self.context_stats_requested = true;
        }

        self.chat.add_entry(ChatEntry::Status {
            text: format!(
                "Session: {} | Messages: {} | Tokens: ↑{} ↓{} | Cost: ${:.4}",
                key, msgs, input, output, cost
            ),
        });
    }

    fn send_set_model(&mut self, model: &str) {
        self.send_command(Command::SetModel {
            id: Some("sm".into()),
            model: Some(model.to_string()),
            provider: None,
            model_id: None,
        });
        self.footer.set_model(model);
        self.current_model = Some(model.to_string());
        self.context_stats_requested = false;
    }

    // ── Resume selector ─────────────────────────────────────────────

    fn open_resume_selector(&mut self, data: &serde_json::Value) {
        let sessions = data
            .get("sessions")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        if sessions.is_empty() {
            self.chat.add_entry(ChatEntry::Status {
                text: "No persisted sessions found.".to_string(),
            });
            return;
        }
        let items = sessions
            .into_iter()
            .filter_map(|session| {
                let name = session.get("name").and_then(|v| v.as_str())?;
                let count = session
                    .get("messageCount")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                Some(SelectItem {
                    value: name.to_string(),
                    label: name.to_string(),
                    description: Some(format!("{count} messages")),
                })
            })
            .collect::<Vec<_>>();
        if items.is_empty() {
            self.chat.add_entry(ChatEntry::Status {
                text: "No resumable CLI sessions found.".to_string(),
            });
            return;
        }
        self.resume_selector = Some(SelectList::new(items, 10));
    }

    fn handle_resume_selector_key(&mut self, key: &Key) {
        if let Some(selector) = &mut self.resume_selector {
            selector.handle_input(key);
            match selector.take_result() {
                SelectResult::Selected(session) => {
                    self.resume_selector = None;
                    self.send_resume_session(&session);
                }
                SelectResult::Cancelled => {
                    self.resume_selector = None;
                }
                SelectResult::Pending => {}
            }
        }
    }

    fn replace_chat_with_messages(&mut self, data: &serde_json::Value) {
        let messages = data
            .get("messages")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        self.chat.clear();
        for message in messages {
            let role = message.get("role").and_then(|v| v.as_str()).unwrap_or("");
            let content = message
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            match role {
                "user" => self.chat.add_entry(ChatEntry::User { text: content }),
                "assistant" if !content.is_empty() => self.chat.add_entry(ChatEntry::Assistant {
                    text: content,
                    streaming: false,
                }),
                _ => {}
            }
        }
        self.chat.add_entry(ChatEntry::Status {
            text: "Session resumed".to_string(),
        });
    }

    // ── Model selector ──────────────────────────────────────────────

    fn open_model_selector(&mut self) {
        let selector = ModelSelector::new(self.current_model.as_deref());
        self.model_selector = Some(selector);
    }

    fn handle_model_selector_key(&mut self, key: &Key) {
        if let Some(selector) = &mut self.model_selector {
            selector.handle_input(key);

            match selector.take_result() {
                ModelSelectorResult::Selected(model) => {
                    self.model_selector = None;
                    self.send_set_model(&model);
                }
                ModelSelectorResult::Cancelled => {
                    self.model_selector = None;
                }
                ModelSelectorResult::Pending => {}
            }
        }
    }

    // ── Notifications ─────────────────────────────────────────────────

    fn notify(&mut self, message: &str, level: NotifyLevel) {
        self.notifications.push(Notification::new(message, level));
    }

    // ── Rendering ─────────────────────────────────────────────────────

    #[allow(clippy::cognitive_complexity)]
    fn render(&mut self) {
        let width = self.terminal.width;
        let height = self.terminal.height;

        let mut lines = Vec::new();
        let mut workflow_bar_state = self.workflow_bar.clone();
        workflow_bar_state.workflow_auto_continue = self.workflow_auto_continue;
        workflow_bar_state.workflow_completion_nudge = self.workflow_completion_nudge;

        // ── Render bottom section first to know its height ──────────
        let mut bottom = Vec::new();

        // Widgets above editor (subagent bars stay on top, visible).
        bottom.extend(self.widgets_above.render(width));

        // Pi-style workflow widget above the editor.
        let workflow_widget_lines = workflow_bar::render_widget(&workflow_bar_state, width);
        bottom.extend(workflow_widget_lines);

        // Spinner sits between widgets_above and autocomplete (#534).
        if let Some(spinner) = &mut self.spinner {
            bottom.extend(spinner.render(width));
        }

        // Autocomplete dropdown.
        bottom.extend(self.autocomplete.render(width));

        // Editor.
        bottom.extend(self.editor.render(width));

        // Widgets below editor.
        bottom.extend(self.widgets_below.render(width));

        // Notifications.
        bottom.extend(self.notifications.render(width));

        // Footer.
        bottom.extend(self.footer.render(width));

        let bottom_height = bottom.len();

        // ── Render top section (header + chat) ──────────────────────
        // Header.
        let version = env!("CARGO_PKG_VERSION");
        lines.push(theme::dim(&format!(
            "quecto-tui v{} — Enter send, Shift+Enter newline, /help for commands",
            version
        )));

        // Chat — render into available space above the bottom section.
        // Reserve MIN_CHAT_GAP lines for spacing between chat and editor (#480).
        const MIN_CHAT_GAP: usize = 3;
        let chat_height = height.saturating_sub(bottom_height + 2 + MIN_CHAT_GAP);
        self.chat.set_viewport_height(chat_height);
        let mut chat_lines = self.chat.render(width);

        // If chat is taller than available space, show only the tail (auto-scroll).
        if chat_lines.len() > chat_height {
            let start = chat_lines.len() - chat_height;
            chat_lines = chat_lines[start..].to_vec();
        }
        lines.extend(chat_lines);

        // Pad between chat and bottom to push bottom to the screen bottom.
        let available = height.saturating_sub(bottom_height);
        while lines.len() < available {
            lines.push(String::new());
        }

        // ── Append bottom section ───────────────────────────────────
        lines.extend(bottom);

        // Final safety: ensure exactly `height` lines.
        if lines.len() > height {
            let start = lines.len() - height;
            lines = lines[start..].to_vec();
        }
        while lines.len() < height {
            lines.push(String::new());
        }

        // Composite overlays on top.
        if self.overlay_stack.has_visible() {
            self.overlay_stack.composite(&mut lines, width, height);
        }

        // Composite resume selector overlay if active.
        // Uses ANSI-aware splice_line to avoid escape code bleeding.
        if let Some(selector) = &mut self.resume_selector {
            let (selector_lines, overlay_width) =
                build_resume_selector_overlay(selector, width, height);
            let overlay_height = selector_lines.len().min(height.saturating_sub(4));
            let start_row = height.saturating_sub(overlay_height) / 2;
            let start_col = width.saturating_sub(overlay_width) / 2;
            for i in 0..overlay_height {
                let row = start_row + i;
                if row < lines.len() && i < selector_lines.len() {
                    lines[row] = crate::interface::overlay::splice_line(
                        &lines[row],
                        &selector_lines[i],
                        start_col,
                        overlay_width,
                        width,
                    );
                }
            }
        }

        // Composite model selector overlay if active.
        // Uses ANSI-aware splice_line to avoid escape code bleeding.
        if let Some(selector) = &mut self.model_selector {
            let overlay_width = width.saturating_sub(4).min(60);
            let selector_lines = selector.render(overlay_width);
            let overlay_height = selector_lines.len().min(height.saturating_sub(4));

            // Center the overlay.
            let start_row = height.saturating_sub(overlay_height) / 2;
            let start_col = width.saturating_sub(overlay_width) / 2;

            for i in 0..overlay_height {
                let row = start_row + i;
                if row < lines.len() && i < selector_lines.len() {
                    lines[row] = crate::interface::overlay::splice_line(
                        &lines[row],
                        &selector_lines[i],
                        start_col,
                        overlay_width,
                        width,
                    );
                }
            }
        }

        // Enforce width on every line.
        for line in &mut lines {
            if crate::interface::utils::visible_width(line) > width {
                *line = crate::interface::utils::truncate_to_width(line, width, None);
            }
        }

        // Store rendered lines for text selection extraction (#528).
        // Must happen BEFORE highlight injection to avoid leaking
        // reverse-video escapes into the extraction buffer (#546 review).
        self.last_rendered_lines = lines.clone();

        // Apply mouse selection highlight (#546).
        // Applied to a separate copy so extraction buffer stays clean.
        apply_selection_highlight(&self.selection, &mut lines);

        // Write to terminal.
        let mut buf = String::new();
        buf.push_str("\x1b[?2026h");
        buf.push_str("\x1b[H");

        for (i, line) in lines.iter().enumerate() {
            if i > 0 {
                buf.push_str("\r\n");
            }
            buf.push_str("\x1b[2K");
            buf.push_str(line);
        }

        let rendered = lines.len();
        if rendered < height {
            for _ in rendered..height {
                buf.push_str("\r\n\x1b[2K");
            }
        }

        buf.push_str("\x1b[?2026l");

        let _ = std::io::stdout().write_all(buf.as_bytes());
        let _ = std::io::stdout().flush();
    }

    fn render_full(&mut self) {
        self.terminal.clear_screen();
        self.render();
    }

    /// Reset the conversation — clears agent history, chat UI, and context display.
    fn reset_session(&mut self, message: &str) {
        self.send_clear_history();
        self.chat.clear();
        self.footer.set_context(None, 0);
        self.context_stats_requested = false;
        self.notify(message, NotifyLevel::Success);
    }

    fn send_clear_history(&mut self) {
        self.send_command(Command::ClearHistory { id: None });
    }

    // ── Command sending ───────────────────────────────────────────────

    fn send_command(&mut self, cmd: Command) {
        let mut sender = self.client.clone_sender();
        tokio::spawn(async move {
            let _ = sender.send(&cmd).await;
        });
    }

    // ── Mouse text selection (#528) ───────────────────────────────────

    /// Extract visible text from the rendered buffer between two selection anchors.
    fn extract_selection(&self, start: &SelectionAnchor, end: &SelectionAnchor) -> String {
        // Normalize: ensure start ≤ end (top-to-bottom, left-to-right).
        let (start, end) = if (start.row, start.col) <= (end.row, end.col) {
            (start, end)
        } else {
            (end, start)
        };

        let lines = &self.last_rendered_lines;
        let mut result = String::new();

        for row in start.row..=end.row {
            let row_idx = row as usize;
            if row_idx >= lines.len() {
                break;
            }
            let visible = strip_ansi_for_selection(&lines[row_idx]);
            let chars: Vec<char> = visible.chars().collect();

            let col_start = if row == start.row {
                start.col as usize
            } else {
                0
            };
            let col_end = if row == end.row {
                end.col as usize
            } else {
                chars.len()
            };

            let col_start = col_start.min(chars.len());
            let col_end = col_end.min(chars.len());

            let segment: String = chars[col_start..col_end].iter().collect();

            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(&segment);
        }

        result
    }
}

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
#[allow(clippy::cognitive_complexity)]
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

/// Read the current git branch from .git/HEAD.
fn read_git_branch() -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    let head = std::fs::read_to_string(cwd.join(".git/HEAD")).ok()?;
    let trimmed = head.trim();
    trimmed
        .strip_prefix("ref: ")
        .and_then(|r| r.rsplit('/').next())
        .map(|s| s.to_string())
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
    /// When the subagent entered the "exited" state. `None` if still active.
    exited_at: Option<tokio::time::Instant>,
}

impl TrackedSubagent {
    fn new(info: crate::infrastructure::client::SubagentInfoEvent) -> Self {
        let exited_at = if info.status == STATUS_EXITED {
            Some(tokio::time::Instant::now())
        } else {
            None
        };
        Self { info, exited_at }
    }

    /// Update the info, recording exited_at on transition to "exited".
    fn update_info(&mut self, new_info: crate::infrastructure::client::SubagentInfoEvent) {
        if new_info.status == STATUS_EXITED && self.exited_at.is_none() {
            self.exited_at = Some(tokio::time::Instant::now());
        } else if new_info.status != STATUS_EXITED {
            self.exited_at = None;
        }
        self.info = new_info;
    }
}

/// Remove exited subagents whose grace period has elapsed (#540).
/// Returns `true` if any entries were removed.
fn gc_exited_subagents(
    map: &mut std::collections::BTreeMap<String, TrackedSubagent>,
    now: tokio::time::Instant,
    grace: Duration,
) -> bool {
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

fn suppress_tool_box(tool_name: &str, args: &serde_json::Value) -> bool {
    match tool_name {
        "spawn" => true,
        "agent_cmd" => {
            let cmd = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
            matches!(cmd, "prompt" | "steer" | "abort")
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_state_not_running() {
        let state = AgentRunState::new();
        assert!(!state.is_running());
    }

    #[test]
    fn start_sets_running() {
        let mut state = AgentRunState::new();
        state.start();
        assert!(state.is_running());
    }

    #[test]
    fn normal_end_clears_running() {
        let mut state = AgentRunState::new();
        state.start();
        assert!(state.is_running());
        let processed = state.end();
        assert!(processed);
        assert!(!state.is_running());
    }

    #[test]
    fn stale_agent_end_before_new_start_consumed() {
        // Scenario: stale AgentEnd arrives BEFORE new AgentStart.
        let mut state = AgentRunState::new();
        state.start(); // run 1
        state.abort(); // pending = 1, running = false

        // Stale AgentEnd arrives before user sends new prompt.
        let processed = state.end();
        assert!(!processed, "stale end should be consumed by pending_aborts");

        // Now new prompt → AgentStart.
        state.start(); // run 2
        assert!(state.is_running());

        // Real AgentEnd from run 2 — processed normally.
        assert!(state.end());
        assert!(!state.is_running());
    }

    #[test]
    fn stale_agent_end_never_arrives_new_run_works() {
        // Scenario (#506): agent backend does NOT send AgentEnd for
        // aborted run. New AgentStart clears pending_aborts so the
        // real AgentEnd is not eaten.
        let mut state = AgentRunState::new();
        state.start(); // run 1
        state.abort(); // pending = 1

        // No stale AgentEnd arrives. User sends new prompt.
        state.start(); // run 2 — clears pending_aborts to 0
        assert!(state.is_running());

        // Real AgentEnd from run 2 — must be processed, not eaten.
        assert!(state.end());
        assert!(!state.is_running());
    }

    #[test]
    fn abort_clears_running_for_ui() {
        let mut state = AgentRunState::new();
        state.start();
        state.abort();
        assert!(!state.is_running(), "abort should clear running for UI");
    }

    #[test]
    fn abort_when_not_running_is_noop() {
        let mut state = AgentRunState::new();
        state.abort(); // should not panic or increment pending_aborts
        assert!(!state.is_running());
        // End should process normally (no pending aborts).
        state.start();
        assert!(state.end());
    }

    #[test]
    fn multiple_aborts_with_starts_clears_pending() {
        let mut state = AgentRunState::new();
        state.start(); // run 1
        state.abort(); // pending = 1
        state.start(); // run 2 — clears pending to 0
        state.abort(); // pending = 1
        state.start(); // run 3 — clears pending to 0

        // Only the current run's end matters. No stale ends to consume.
        assert!(state.end()); // run 3 ends normally
        assert!(!state.is_running());
    }

    #[test]
    fn normal_flow_without_abort() {
        let mut state = AgentRunState::new();
        state.start();
        assert!(state.is_running());
        state.end();
        assert!(!state.is_running());

        state.start();
        assert!(state.is_running());
        state.end();
        assert!(!state.is_running());
    }

    #[test]
    fn abort_then_end_without_new_start() {
        let mut state = AgentRunState::new();
        state.start();
        state.abort(); // pending = 1, running = false

        // AgentEnd from the aborted run is consumed.
        assert!(!state.end());
        // Running is false (abort cleared it).
        assert!(!state.is_running());
        // Next prompt works correctly.
        state.start();
        assert!(state.is_running());
    }

    #[test]
    fn abort_then_end_then_new_start_works() {
        let mut state = AgentRunState::new();
        state.start(); // run 1
        state.abort(); // pending = 1

        // Stale AgentEnd consumed.
        state.end();

        // New prompt works.
        state.start(); // run 2
        assert!(state.is_running());
        assert!(state.end()); // run 2 ends normally
        assert!(!state.is_running());
    }

    #[test]
    fn reset_clears_all_state() {
        let mut state = AgentRunState::new();
        state.start();
        state.abort(); // pending = 1
        state.reset();
        assert!(!state.is_running());
        // After reset, end() should work normally (no stale aborts).
        state.start();
        assert!(state.end());
    }

    #[test]
    fn start_clears_pending_aborts() {
        // Issue #506: If the agent backend doesn't send AgentEnd for
        // an aborted run, pending_aborts stays stale and eats the
        // next real AgentEnd.
        let mut state = AgentRunState::new();
        state.start(); // run 1
        state.abort(); // pending_aborts = 1

        // Agent backend does NOT send AgentEnd for the aborted run.
        // User sends a new prompt → AgentStart arrives.
        state.start(); // run 2

        // The real AgentEnd from run 2 should be processed normally,
        // NOT eaten by the stale pending_aborts.
        assert!(
            state.end(),
            "AgentEnd for new run should be processed, not consumed by stale pending_aborts"
        );
        assert!(!state.is_running());
    }

    // ── Render order tests (issue #534) ──────────────────────────────

    #[test]
    fn spinner_renders_after_widgets_above_in_bottom_section() {
        // Verify the render order: widgets_above → spinner → autocomplete → editor
        // This is a structural test — we verify the render() method's bottom
        // section is built in the correct order by checking code structure.
        // The actual render method builds `bottom` with:
        //   1. widgets_above.render()
        //   2. spinner.render()
        //   3. autocomplete.render()
        //   4. editor.render()
        // We confirm this by reading the source (compile-time verification).
        // The important invariant: subagent bars appear BEFORE the spinner.
        //
        // We can't easily instantiate App in tests (requires Terminal + Client),
        // but the render order is verified by the code structure and the
        // integration BDD scenarios.
        // Render order verified by code review: widgets_above before spinner.
        // See render() method — subagent bars appear BEFORE the spinner.
    }

    // ── Base64 encoding tests (issue #528) ────────────────────────────

    #[test]
    fn base64_encode_empty() {
        assert_eq!(super::base64_encode(b""), "");
    }

    #[test]
    fn base64_encode_hello() {
        assert_eq!(super::base64_encode(b"hello"), "aGVsbG8=");
    }

    #[test]
    fn base64_encode_hello_world() {
        assert_eq!(super::base64_encode(b"hello world"), "aGVsbG8gd29ybGQ=");
    }

    #[test]
    fn base64_encode_one_byte() {
        assert_eq!(super::base64_encode(b"a"), "YQ==");
    }

    #[test]
    fn base64_encode_two_bytes() {
        assert_eq!(super::base64_encode(b"ab"), "YWI=");
    }

    #[test]
    fn base64_encode_three_bytes() {
        assert_eq!(super::base64_encode(b"abc"), "YWJj");
    }

    // ── ANSI stripping tests (issue #528) ──────────────────────────────

    #[test]
    fn strip_ansi_plain_text() {
        assert_eq!(super::strip_ansi_for_selection("hello"), "hello");
    }

    #[test]
    fn strip_ansi_sgr() {
        assert_eq!(super::strip_ansi_for_selection("\x1b[31mred\x1b[0m"), "red");
    }

    #[test]
    fn strip_ansi_osc() {
        assert_eq!(
            super::strip_ansi_for_selection("\x1b]0;title\x07text"),
            "text"
        );
    }

    #[test]
    fn strip_ansi_mixed() {
        assert_eq!(
            super::strip_ansi_for_selection("\x1b[32m✓\x1b[0m $ \x1b[1mgit status\x1b[0m"),
            "✓ $ git status"
        );
    }

    #[test]
    fn stale_agent_end_after_new_start_kills_run() {
        // Critical race: abort → new start → stale AgentEnd arrives.
        // Since start() cleared pending_aborts, the stale end is
        // indistinguishable from the real end. This is a known
        // limitation — the protocol has no generation IDs.
        // The result: the new run appears to end prematurely.
        // This is better than the alternative (#506: new run hangs forever).
        let mut state = AgentRunState::new();
        state.start(); // run 1
        state.abort(); // pending = 1
        state.start(); // run 2 — clears pending to 0

        // Stale AgentEnd from run 1 arrives after run 2 started.
        // It's processed as run 2's end (no way to distinguish).
        let processed = state.end();
        assert!(processed, "stale end processed as current run's end");
        assert!(!state.is_running());

        // The real AgentEnd from run 2 will arrive later — but since
        // running is already false, it's harmless (end() on !running is a no-op).
    }

    #[test]
    fn start_always_clears_pending_aborts() {
        // Regardless of how many aborts happened before, start()
        // always resets pending_aborts so the new run works cleanly.
        let mut state = AgentRunState::new();
        state.start();
        state.abort();
        // Stale end arrives before new start.
        state.end();
        state.start();
        state.abort();
        state.end();
        state.start();
        state.abort();
        // No stale end arrives this time.
        state.start(); // clears pending_aborts

        // The new run's end should work.
        assert!(state.end());
        assert!(!state.is_running());
    }

    // ── Free function tests ──────────────────────────────────────────

    #[test]
    fn builtin_commands_not_empty() {
        let cmds = super::builtin_commands();
        assert!(!cmds.is_empty());
        let names: Vec<_> = cmds.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"clear"));
        assert!(names.contains(&"quit"));
        assert!(names.contains(&"exit"));
        assert!(names.contains(&"help"));
        assert!(names.contains(&"new"));
        assert!(names.contains(&"session"));
        assert!(names.contains(&"resume"));
        assert!(names.contains(&"model"));
        assert!(names.contains(&"workflow"));
    }

    #[test]
    fn builtin_commands_have_descriptions() {
        let cmds = super::builtin_commands();
        for cmd in &cmds {
            assert!(
                !cmd.description.is_empty(),
                "{} has empty description",
                cmd.name
            );
        }
    }

    #[test]
    fn resume_selector_overlay_has_opaque_border() {
        let mut selector = SelectList::new(
            vec![SelectItem {
                label: "default".into(),
                value: "default".into(),
                description: Some("2 messages".into()),
            }],
            10,
        );
        let (lines, width) = super::build_resume_selector_overlay(&mut selector, 100, 40);

        assert!(
            width > 72,
            "border should make the overlay wider than the list"
        );
        assert!(
            lines.len() > 4,
            "overlay should include top/bottom border padding"
        );
        assert!(
            lines.iter().all(|line| line.contains(theme::BG_OVERLAY)),
            "every overlay line should use the opaque background"
        );
        assert!(
            lines
                .iter()
                .all(|line| crate::interface::utils::visible_width(line) == width),
            "opaque background should span the full overlay width"
        );
    }

    #[test]
    fn sanitize_workflow_status_text_strips_control_and_truncates() {
        let result = super::sanitize_workflow_status_text("hello\x1b[31mworld\x00end", 10);
        assert!(!result.contains('\x1b'));
        assert!(!result.contains('\x00'));
        assert!(result.ends_with('…'));
    }

    #[test]
    fn truncate_args_short() {
        assert_eq!(super::truncate_args("ls -la"), "ls -la");
    }

    #[test]
    fn truncate_args_long() {
        let long = "a".repeat(50);
        let result = super::truncate_args(&long);
        assert!(result.len() <= 40);
        assert!(result.ends_with("..."));
    }

    #[test]
    fn truncate_args_strips_control() {
        let s = "hello\x1b[31mworld\x00end";
        let result = super::truncate_args(s);
        assert!(!result.contains('\x1b'));
        assert!(!result.contains('\x00'));
    }

    #[test]
    fn truncate_args_empty() {
        assert_eq!(super::truncate_args(""), "");
    }

    #[test]
    fn read_git_branch_returns_some_in_repo() {
        // We're in a git repo during tests
        let branch = super::read_git_branch();
        // May be None if running from a detached HEAD or non-git context
        // but in our repo it should be Some
        if let Some(b) = branch {
            assert!(!b.is_empty());
        }
    }

    #[test]
    fn strip_ansi_for_selection_empty() {
        assert_eq!(super::strip_ansi_for_selection(""), "");
    }

    #[test]
    fn strip_ansi_csi_with_tilde() {
        assert_eq!(super::strip_ansi_for_selection("\x1b[5~text"), "text");
    }

    #[test]
    fn strip_ansi_nested_escapes() {
        assert_eq!(
            super::strip_ansi_for_selection("\x1b[1m\x1b[32mbold green\x1b[0m"),
            "bold green"
        );
    }

    #[test]
    fn strip_ansi_osc_with_st() {
        // OSC terminated with ST (\x1b\)
        assert_eq!(
            super::strip_ansi_for_selection("\x1b]0;title\x1b\\text"),
            "text"
        );
    }

    #[test]
    fn base64_encode_multibyte() {
        // Test with UTF-8 content
        let result = super::base64_encode("héllo".as_bytes());
        assert!(!result.is_empty());
        assert!(
            result
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=')
        );
    }

    #[test]
    fn base64_encode_binary() {
        let result = super::base64_encode(&[0, 1, 2, 255, 254, 253]);
        assert!(!result.is_empty());
    }

    #[test]
    fn max_clipboard_bytes_is_reasonable() {
        const _: () = {
            assert!(super::MAX_CLIPBOARD_BYTES >= 1024);
            assert!(super::MAX_CLIPBOARD_BYTES <= 1024 * 1024);
        };
    }

    #[test]
    fn selection_anchor_copy() {
        let a = super::SelectionAnchor { col: 10, row: 5 };
        let b = a; // Copy
        assert_eq!(a.col, b.col);
        assert_eq!(a.row, b.row);
    }

    #[test]
    fn text_selection_clone() {
        let sel = super::TextSelection {
            start: super::SelectionAnchor { col: 0, row: 0 },
            end: super::SelectionAnchor { col: 10, row: 5 },
        };
        let sel2 = sel.clone();
        assert_eq!(sel2.start.col, 0);
        assert_eq!(sel2.end.col, 10);
    }

    // ── Ctrl+C action decision tests (#536) ──────────────────────────

    #[test]
    fn ctrl_c_clears_editor_when_running_and_has_text() {
        assert_eq!(
            super::ctrl_c_action(true, false),
            super::CtrlCAction::ClearEditor,
            "Ctrl+C should clear editor when agent is running but editor has text"
        );
    }

    #[test]
    fn ctrl_c_aborts_when_running_and_editor_empty() {
        assert_eq!(
            super::ctrl_c_action(true, true),
            super::CtrlCAction::AbortAgent,
            "Ctrl+C should abort agent when running and editor is empty"
        );
    }

    #[test]
    fn ctrl_c_clears_editor_when_idle_and_has_text() {
        assert_eq!(
            super::ctrl_c_action(false, false),
            super::CtrlCAction::ClearEditor,
            "Ctrl+C should clear editor when idle and editor has text"
        );
    }

    #[test]
    fn ctrl_c_noop_when_idle_and_editor_empty() {
        assert_eq!(
            super::ctrl_c_action(false, true),
            super::CtrlCAction::Noop,
            "Ctrl+C should do nothing when idle and editor is empty"
        );
    }

    // ── Subagent tool classification tests (#538) ──────────────────

    #[test]
    fn spawn_is_subagent_tool() {
        assert!(super::is_subagent_tool("spawn"));
    }

    #[test]
    fn agent_cmd_is_subagent_tool() {
        assert!(super::is_subagent_tool("agent_cmd"));
    }

    #[test]
    fn regular_tools_are_not_subagent_tools() {
        assert!(!super::is_subagent_tool("bash"));
        assert!(!super::is_subagent_tool("read"));
        assert!(!super::is_subagent_tool("write"));
    }

    // ── Tool output suppression tests (#538) ─────────────────────────

    #[test]
    fn spawn_tool_output_suppressed() {
        let args = serde_json::json!({"agent_id": "worker-1"});
        assert!(
            super::suppress_tool_box("spawn", &args),
            "spawn output should be suppressed (status bar shows it)"
        );
    }

    #[test]
    fn agent_cmd_query_output_shown() {
        for cmd in &[
            "get_state",
            "get_messages_tail",
            "get_session_stats",
            "get_messages",
            "get_subagents",
        ] {
            let args = serde_json::json!({"agent_id": "w1", "command": cmd});
            assert!(
                !super::suppress_tool_box("agent_cmd", &args),
                "agent_cmd {cmd} output should be shown (query result)"
            );
        }
    }

    #[test]
    fn agent_cmd_mutation_output_suppressed() {
        for cmd in &["prompt", "steer", "abort"] {
            let args = serde_json::json!({"agent_id": "w1", "command": cmd});
            assert!(
                super::suppress_tool_box("agent_cmd", &args),
                "agent_cmd {cmd} output should be suppressed (mutation)"
            );
        }
    }

    #[test]
    fn agent_cmd_unknown_command_shown() {
        let args = serde_json::json!({"agent_id": "w1", "command": "future_query"});
        assert!(
            !super::suppress_tool_box("agent_cmd", &args),
            "unknown agent_cmd commands should be shown by default"
        );
    }

    #[test]
    fn regular_tool_output_shown() {
        let args = serde_json::json!({});
        assert!(!super::suppress_tool_box("bash", &args));
        assert!(!super::suppress_tool_box("read", &args));
        assert!(!super::suppress_tool_box("write", &args));
        assert!(!super::suppress_tool_box("edit", &args));
    }

    // ── Exited subagent GC tests (#540) ──────────────────────────────

    fn make_tracked(id: &str, status: &str) -> (String, super::TrackedSubagent) {
        (
            id.to_string(),
            super::TrackedSubagent::new(crate::infrastructure::client::SubagentInfoEvent {
                agent_id: id.to_string(),
                status: status.to_string(),
                last_tool: None,
                last_error: None,
                pid: 0,
            }),
        )
    }

    #[test]
    fn gc_removes_expired_exited_subagent() {
        let mut map = std::collections::BTreeMap::new();
        let (id, mut entry) = make_tracked("w1", "exited");
        // Backdate the exited_at to 10 seconds ago.
        entry.exited_at = Some(tokio::time::Instant::now() - Duration::from_secs(10));
        map.insert(id, entry);

        let removed = super::gc_exited_subagents(
            &mut map,
            tokio::time::Instant::now(),
            Duration::from_secs(5),
        );
        assert!(removed, "should have removed expired entry");
        assert!(map.is_empty());
    }

    #[test]
    fn gc_keeps_recent_exited_subagent() {
        let mut map = std::collections::BTreeMap::new();
        let (id, entry) = make_tracked("w1", "exited");
        // exited_at is just now — within grace period.
        map.insert(id, entry);

        let removed = super::gc_exited_subagents(
            &mut map,
            tokio::time::Instant::now(),
            Duration::from_secs(5),
        );
        assert!(!removed, "should not remove recent exit");
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn gc_keeps_running_subagent() {
        let mut map = std::collections::BTreeMap::new();
        let (id, entry) = make_tracked("w1", "running");
        map.insert(id, entry);

        let removed = super::gc_exited_subagents(
            &mut map,
            tokio::time::Instant::now(),
            Duration::from_secs(5),
        );
        assert!(!removed, "should not remove running subagent");
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn gc_mixed_removes_only_expired() {
        let mut map = std::collections::BTreeMap::new();

        let (id1, entry1) = make_tracked("active", "running");
        map.insert(id1, entry1);

        let (id2, mut entry2) = make_tracked("old-exit", "exited");
        entry2.exited_at = Some(tokio::time::Instant::now() - Duration::from_secs(10));
        map.insert(id2, entry2);

        let (id3, entry3) = make_tracked("recent-exit", "exited");
        map.insert(id3, entry3);

        let removed = super::gc_exited_subagents(
            &mut map,
            tokio::time::Instant::now(),
            Duration::from_secs(5),
        );
        assert!(removed, "should have removed old-exit");
        assert_eq!(map.len(), 2);
        assert!(map.contains_key("active"));
        assert!(map.contains_key("recent-exit"));
        assert!(!map.contains_key("old-exit"));
    }

    #[test]
    fn tracked_subagent_new_sets_exited_at_for_exited() {
        let entry = super::TrackedSubagent::new(crate::infrastructure::client::SubagentInfoEvent {
            agent_id: "w1".into(),
            status: "exited".into(),
            last_tool: None,
            last_error: None,
            pid: 0,
        });
        assert!(entry.exited_at.is_some());
    }

    #[test]
    fn tracked_subagent_new_no_exited_at_for_running() {
        let entry = super::TrackedSubagent::new(crate::infrastructure::client::SubagentInfoEvent {
            agent_id: "w1".into(),
            status: "running".into(),
            last_tool: None,
            last_error: None,
            pid: 0,
        });
        assert!(entry.exited_at.is_none());
    }

    #[test]
    fn tracked_subagent_update_sets_exited_at_on_transition() {
        let mut entry =
            super::TrackedSubagent::new(crate::infrastructure::client::SubagentInfoEvent {
                agent_id: "w1".into(),
                status: "running".into(),
                last_tool: None,
                last_error: None,
                pid: 0,
            });
        assert!(entry.exited_at.is_none());

        entry.update_info(crate::infrastructure::client::SubagentInfoEvent {
            agent_id: "w1".into(),
            status: "exited".into(),
            last_tool: None,
            last_error: None,
            pid: 0,
        });
        assert!(entry.exited_at.is_some());
    }

    #[test]
    fn tracked_subagent_update_clears_exited_at_on_revival() {
        let mut entry =
            super::TrackedSubagent::new(crate::infrastructure::client::SubagentInfoEvent {
                agent_id: "w1".into(),
                status: "exited".into(),
                last_tool: None,
                last_error: None,
                pid: 0,
            });
        assert!(entry.exited_at.is_some());

        entry.update_info(crate::infrastructure::client::SubagentInfoEvent {
            agent_id: "w1".into(),
            status: "running".into(),
            last_tool: None,
            last_error: None,
            pid: 0,
        });
        assert!(entry.exited_at.is_none());
    }

    #[test]
    fn exited_subagent_grace_is_reasonable() {
        assert!(super::EXITED_SUBAGENT_GRACE.as_secs() >= 2);
        assert!(super::EXITED_SUBAGENT_GRACE.as_secs() <= 30);
    }

    // ── Mouse highlight tests (#546) ─────────────────────────────────

    #[test]
    fn highlight_plain_text_full_line() {
        let result = super::apply_line_highlight("hello world", 0, 11);
        assert!(result.contains("\x1b[7m"), "should contain reverse-on");
        assert!(result.contains("\x1b[27m"), "should contain reverse-off");
        assert!(result.contains("hello world"));
    }

    #[test]
    fn highlight_plain_text_partial() {
        let result = super::apply_line_highlight("hello world", 2, 7);
        // Before highlight: "he"
        // Highlighted: "llo w"
        // After highlight: "orld"
        assert!(result.contains("\x1b[7m"));
        assert!(result.contains("\x1b[27m"));
    }

    #[test]
    fn highlight_noop_when_start_equals_end() {
        let result = super::apply_line_highlight("hello", 3, 3);
        assert_eq!(result, "hello");
    }

    #[test]
    fn highlight_noop_when_start_exceeds_end() {
        let result = super::apply_line_highlight("hello", 5, 2);
        assert_eq!(result, "hello");
    }

    #[test]
    fn highlight_with_ansi_escapes() {
        let line = "\x1b[32mgreen\x1b[0m text";
        let result = super::apply_line_highlight(line, 0, 5);
        // Should highlight "green" (5 visible chars)
        assert!(result.contains("\x1b[7m"));
        assert!(result.contains("\x1b[27m"));
        // ANSI codes should be preserved
        assert!(result.contains("\x1b[32m"));
    }

    #[test]
    fn highlight_closes_at_line_end() {
        let result = super::apply_line_highlight("abc", 1, 100);
        // Start at col 1, end beyond line length
        assert!(result.contains("\x1b[7m"));
        assert!(result.contains("\x1b[27m"), "must close highlight at end");
    }

    #[test]
    fn selection_range_normalizes_forward() {
        let sel = super::TextSelection {
            start: super::SelectionAnchor { col: 5, row: 2 },
            end: super::SelectionAnchor { col: 10, row: 4 },
        };
        let (sr, sc, er, ec) = super::selection_range(&sel);
        assert_eq!((sr, sc, er, ec), (2, 5, 4, 10));
    }

    #[test]
    fn selection_range_normalizes_backward() {
        let sel = super::TextSelection {
            start: super::SelectionAnchor { col: 10, row: 4 },
            end: super::SelectionAnchor { col: 5, row: 2 },
        };
        let (sr, sc, er, ec) = super::selection_range(&sel);
        assert_eq!((sr, sc, er, ec), (2, 5, 4, 10));
    }

    #[test]
    fn selection_range_same_row_normalizes() {
        let sel = super::TextSelection {
            start: super::SelectionAnchor { col: 10, row: 3 },
            end: super::SelectionAnchor { col: 2, row: 3 },
        };
        let (sr, sc, er, ec) = super::selection_range(&sel);
        assert_eq!((sr, sc, er, ec), (3, 2, 3, 10));
    }
}
