//! Application — the main TUI event loop.
//!
//! Wires all components together: terminal, UDS client, editor, chat,
//! spinner, footer, autocomplete, overlays, notifications, Kitty protocol,
//! signal handling, and extension management.

use std::io::Write;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::client::{Client, Command, Event};
use crate::component::Component;
use crate::components::autocomplete::{Autocomplete, AutocompleteResult, SlashCommand};
use crate::components::chat::{Chat, ChatEntry};
use crate::components::editor::Editor;
use crate::components::footer::Footer;
use crate::components::notification::{Notification, NotificationStack, NotifyLevel};
use crate::components::spinner::Spinner;
use crate::components::widget::WidgetContainer;
use crate::keys::{self, Key};
use crate::kitty::KittyProtocol;
use crate::overlay::OverlayStack;
use crate::terminal::Terminal;
use crate::theme;

/// Tick interval for spinner animation (~12fps).
const SPINNER_TICK: Duration = Duration::from_millis(80);

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
            name: "new".into(),
            description: "Start a new session".into(),
        },
        SlashCommand {
            name: "session".into(),
            description: "Show session info".into(),
        },
        SlashCommand {
            name: "model".into(),
            description: "Switch model".into(),
        },
    ]
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
    /// Whether the agent is currently processing.
    agent_running: bool,
    /// Whether the app should exit.
    should_exit: bool,
    /// Track tool output expansion state.
    tool_expanded: bool,
    /// Stdin input buffer — accumulates bytes for escape sequence parsing.
    stdin_buf: Vec<u8>,
    /// Whether the agent connection is still alive.
    agent_connected: bool,
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
            agent_running: false,
            should_exit: false,
            tool_expanded: false,
            stdin_buf: Vec::new(),
            agent_connected: true,
        }
    }

    /// Run the main event loop. Returns exit code.
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

        // Set up SIGWINCH handler.
        let mut resize_rx = crate::signals::sigwinch_stream().await;

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

        // Escape sequence timeout — if we get a bare \x1b, wait this long
        // for more bytes before treating it as a standalone Escape key.
        let escape_timeout = Duration::from_millis(20);

        // Main event loop.
        loop {
            if self.should_exit {
                break;
            }

            tokio::select! {
                // Stdin input.
                Some(bytes) = stdin_rx.recv() => {
                    // Check for Kitty protocol response before key parsing.
                    if !self.kitty.active && !kitty_fallback_done {
                        if let Some(_flags) = KittyProtocol::parse_response(&bytes) {
                            self.kitty.enable();
                            kitty_fallback_done = true;
                            continue;
                        }
                    }
                    // Filter Kitty key release events.
                    if self.kitty.active && crate::kitty::is_key_release(&bytes) {
                        continue;
                    }
                    // Buffer incoming bytes.
                    self.stdin_buf.extend_from_slice(&bytes);

                    // If buffer contains an incomplete escape sequence, wait briefly
                    // for more bytes. This handles split reads where \x1b arrives
                    // separately from [A, or \x1b[ arrives without the terminator.
                    while self.looks_incomplete() {
                        match tokio::time::timeout(escape_timeout, stdin_rx.recv()).await {
                            Ok(Some(more)) => {
                                self.stdin_buf.extend_from_slice(&more);
                            }
                            _ => break, // Timeout or closed — process what we have.
                        }
                    }

                    // Parse all buffered keys.
                    self.handle_stdin_buf();
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
                            self.agent_running = false;
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

    /// Check if the stdin buffer ends with an incomplete escape sequence.
    fn looks_incomplete(&self) -> bool {
        let buf = &self.stdin_buf;
        if buf.is_empty() {
            return false;
        }
        // Bare \x1b at end — might be start of escape sequence.
        if *buf.last().unwrap() == 0x1b {
            return true;
        }
        // \x1b[ without a terminating letter — incomplete CSI.
        if buf.len() >= 2 {
            let tail_start = buf.iter().rposition(|&b| b == 0x1b);
            if let Some(esc_pos) = tail_start {
                let after_esc = &buf[esc_pos + 1..];
                if after_esc.first() == Some(&b'[') {
                    // CSI started — check if it has a terminator (letter or ~).
                    let rest = &after_esc[1..];
                    let has_terminator = rest.iter().any(|&b| b.is_ascii_alphabetic() || b == b'~');
                    return !has_terminator;
                }
                if after_esc.first() == Some(&b'O') && after_esc.len() < 2 {
                    // SS3 started but no character yet.
                    return true;
                }
            }
        }
        false
    }

    fn handle_stdin_buf(&mut self) {
        let mut offset = 0;
        let buf = std::mem::take(&mut self.stdin_buf);
        while offset < buf.len() {
            match keys::parse_key(&buf[offset..]) {
                Some((key, consumed)) => {
                    offset += consumed;
                    self.handle_key(key);
                }
                None => {
                    // Incomplete sequence — put remainder back in buffer.
                    self.stdin_buf = buf[offset..].to_vec();
                    return;
                }
            }
        }
    }

    fn handle_key(&mut self, key: Key) {
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
                    // If autocomplete is active and user presses Enter,
                    // accept the highlighted suggestion first.
                    self.autocomplete.handle_input(&Key::Tab);
                    if let AutocompleteResult::Selected(value) = self.autocomplete.take_result() {
                        self.editor.set_text(&value);
                    }
                    // Don't submit yet — let user review the selected command.
                    return;
                }
                _ => {
                    // Other keys go to editor, then update autocomplete.
                }
            }
        }

        // Global key handlers.
        match &key {
            Key::Ctrl('d') => {
                self.should_exit = true;
                return;
            }
            Key::Ctrl('c') => {
                if self.agent_running {
                    self.handle_abort();
                } else {
                    self.editor.set_text("");
                    self.autocomplete.dismiss();
                }
                return;
            }
            Key::Escape => {
                if self.agent_running {
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
                crate::signals::suspend();
                // Resumed — re-enter raw mode.
                self.terminal.enter_raw_mode();
                self.terminal.hide_cursor();
                self.kitty.query();
                self.render_full();
                return;
            }
            Key::Ctrl('o') => {
                // Toggle tool output expansion.
                self.tool_expanded = !self.tool_expanded;
                let state = if self.tool_expanded {
                    "expanded"
                } else {
                    "collapsed"
                };
                self.notify(&format!("Tool output {}", state), NotifyLevel::Info);
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
                    self.send_clear_history();
                    self.chat.clear();
                    self.notify("Conversation cleared", NotifyLevel::Success);
                    return;
                }
                "/new" => {
                    self.send_clear_history();
                    self.chat.clear();
                    self.notify("New session started", NotifyLevel::Success);
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
                _ if trimmed.starts_with("/model") => {
                    let model_name = trimmed.strip_prefix("/model").unwrap().trim();
                    if !model_name.is_empty() {
                        self.send_set_model(model_name);
                    } else {
                        self.notify("Usage: /model <name>", NotifyLevel::Info);
                    }
                    return;
                }
                _ => {
                    // Unknown slash command — send as regular prompt.
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
            streaming_behavior: if self.agent_running {
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

        // Immediately update local UI state — don't wait for agent_end event.
        self.agent_running = false;
        self.footer.set_streaming(false);

        // Stop spinner.
        self.spinner = None;

        // Finalize any streaming assistant message.
        self.chat.finalize_assistant();

        // Show abort status.
        self.chat.add_entry(ChatEntry::Status {
            text: "Operation aborted".to_string(),
        });
    }

    // ── Agent event handling ──────────────────────────────────────────

    fn handle_event(&mut self, event: Event) {
        match event {
            Event::AgentStart => {
                self.agent_running = true;
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
                    if total > 0 {
                        // Rough context estimate: input tokens as percentage of a 200k window.
                        let window = 200_000usize;
                        let pct = (input as f64 / window as f64) * 100.0;
                        self.footer.set_context(Some(pct), window);
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
                    spinner.set_message(&format!("{} {}...", tool_name, truncate_args(&args_str)));
                }
                self.chat.add_entry(ChatEntry::ToolStart {
                    tool_call_id,
                    tool_name,
                    args: args_str,
                });
            }
            Event::ToolExecutionEnd {
                tool_call_id,
                result,
                is_error,
                ..
            } => {
                let result_text = result
                    .get("content")
                    .and_then(|c| c.as_array())
                    .and_then(|arr| {
                        arr.iter()
                            .filter_map(|v| v.get("text").and_then(|t| t.as_str()))
                            .next()
                    })
                    .unwrap_or("")
                    .to_string();
                self.chat
                    .complete_tool(&tool_call_id, &result_text, is_error, None);
                // Restore spinner message after tool completes.
                if let Some(spinner) = &mut self.spinner {
                    spinner.set_message("Working... (Esc to interrupt)");
                }
            }
            Event::AgentEnd { .. } => {
                self.agent_running = false;
                self.footer.set_streaming(false);
                self.spinner = None;
                self.chat.finalize_assistant();
            }
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
                            self.footer.set_model(model);
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
                "clear_history" if success => {}
                "agent_error" => {
                    let msg = error.unwrap_or_else(|| "unknown error".into());
                    self.chat.add_entry(ChatEntry::Status {
                        text: format!("Error: {}", msg),
                    });
                    self.agent_running = false;
                    self.footer.set_streaming(false);
                    self.spinner = None;
                }
                _ => {}
            },
            _ => {}
        }
    }

    // ── Slash command handlers ─────────────────────────────────────────

    fn show_help(&mut self) {
        self.chat.add_entry(ChatEntry::Status {
            text: [
                "Keyboard shortcuts:",
                "  Enter          Send message",
                "  Shift+Enter    Insert newline",
                "  Escape         Abort agent / clear editor",
                "  Ctrl+C         Clear editor / abort agent",
                "  Ctrl+D         Exit",
                "  Ctrl+Z         Suspend (resume with fg)",
                "  Ctrl+O         Toggle tool output expansion",
                "  PageUp/Down    Scroll chat",
                "  Up/Down        Input history",
                "",
                "Slash commands:",
                "  /model <name>  Switch model",
                "  /clear         Clear conversation",
                "  /new           New session",
                "  /session       Show session info",
                "  /help          This help",
                "  /quit          Exit",
            ]
            .join("\n"),
        });
    }

    fn send_session_stats(&mut self) {
        self.send_command(Command::GetSessionStats {
            id: Some("stats".into()),
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
    }

    // ── Notifications ─────────────────────────────────────────────────

    fn notify(&mut self, message: &str, level: NotifyLevel) {
        self.notifications.push(Notification::new(message, level));
    }

    // ── Rendering ─────────────────────────────────────────────────────

    fn render(&mut self) {
        let width = self.terminal.width;
        let height = self.terminal.height;

        let mut lines = Vec::new();

        // ── Render bottom section first to know its height ──────────
        let mut bottom = Vec::new();

        // Spinner sits above editor (visible during agent work).
        if let Some(spinner) = &mut self.spinner {
            bottom.extend(spinner.render(width));
        }

        // Widgets above editor.
        bottom.extend(self.widgets_above.render(width));

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
        lines.push(String::new());

        // Chat — render into available space above the bottom section.
        let chat_height = height.saturating_sub(bottom_height + 2); // 2 = header lines
        let mut chat_lines = self.chat.render(width);

        // If chat is taller than available space, show only the tail (auto-scroll).
        if chat_lines.len() > chat_height {
            let start = chat_lines.len() - chat_height;
            chat_lines = chat_lines[start..].to_vec();
        }
        lines.extend(chat_lines);

        // Pad between chat and bottom to push bottom to the screen bottom.
        let content_height = lines.len();
        let available = height.saturating_sub(bottom_height);
        if content_height < available {
            for _ in 0..(available - content_height) {
                lines.push(String::new());
            }
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

        // Enforce width on every line.
        for line in &mut lines {
            if crate::utils::visible_width(line) > width {
                *line = crate::utils::truncate_to_width(line, width, None);
            }
        }

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
