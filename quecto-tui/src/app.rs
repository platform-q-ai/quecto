//! Application — the main TUI event loop.
//!
//! Wires together: terminal (raw mode), UDS client (agent communication),
//! editor (user input), chat (message display), spinner (activity indicator),
//! footer (status bar), and the differential renderer.

use std::io::Write;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::client::{Client, Command, Event};
use crate::component::Component;
use crate::components::chat::{Chat, ChatEntry};
use crate::components::editor::Editor;
use crate::components::footer::Footer;
use crate::components::spinner::Spinner;
use crate::keys::{self, Key};
use crate::terminal::Terminal;
use crate::theme;

/// Tick interval for spinner animation (~12fps).
const SPINNER_TICK: Duration = Duration::from_millis(80);

/// Application state.
pub struct App {
    terminal: Terminal,
    client: Client,
    editor: Editor,
    chat: Chat,
    spinner: Option<Spinner>,
    footer: Footer,
    /// Whether the agent is currently processing.
    agent_running: bool,
    /// Whether the app should exit.
    should_exit: bool,
}

impl App {
    pub fn new(terminal: Terminal, client: Client) -> Self {
        let mut footer = Footer::new();
        // Read git branch from .git/HEAD
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
            agent_running: false,
            should_exit: false,
        }
    }

    /// Run the main event loop. Returns exit code.
    pub async fn run(&mut self) -> i32 {
        self.terminal.enter_raw_mode();
        self.terminal.hide_cursor();

        // Query initial state from agent.
        let _ = self
            .client
            .send(&Command::GetState {
                id: Some("init".into()),
            })
            .await;

        // Set up SIGWINCH handler.
        let (resize_tx, mut resize_rx) = mpsc::channel::<()>(1);
        #[cfg(unix)]
        {
            let tx = resize_tx.clone();
            tokio::spawn(async move {
                let mut sig =
                    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change())
                        .expect("failed to register SIGWINCH");
                loop {
                    sig.recv().await;
                    let _ = tx.send(()).await;
                }
            });
        }

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

        // Initial render.
        self.render();

        // Spinner tick timer.
        let mut spinner_interval = tokio::time::interval(SPINNER_TICK);
        spinner_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        // Main event loop.
        loop {
            if self.should_exit {
                break;
            }

            tokio::select! {
                // Stdin input.
                Some(bytes) = stdin_rx.recv() => {
                    self.handle_stdin(&bytes);
                    self.render();
                }
                // Agent events.
                Some(event) = self.client.recv() => {
                    self.handle_event(event);
                    self.render();
                }
                // Terminal resize.
                Some(()) = resize_rx.recv() => {
                    self.terminal.refresh_size();
                    self.render_full();
                }
                // Spinner tick.
                _ = spinner_interval.tick() => {
                    if let Some(spinner) = &mut self.spinner {
                        if spinner.tick() {
                            self.render();
                        }
                    }
                }
            }
        }

        // Cleanup.
        self.terminal.show_cursor();
        self.terminal.exit_raw_mode();
        // Move cursor below content.
        self.terminal.write_str("\r\n");
        0
    }

    // ── Input handling ────────────────────────────────────────────────

    fn handle_stdin(&mut self, bytes: &[u8]) {
        let mut offset = 0;
        while offset < bytes.len() {
            match keys::parse_key(&bytes[offset..]) {
                Some((key, consumed)) => {
                    offset += consumed;
                    self.handle_key(key);
                }
                None => break,
            }
        }
    }

    fn handle_key(&mut self, key: Key) {
        match &key {
            // Ctrl+D — exit.
            Key::Ctrl('d') => {
                self.should_exit = true;
                return;
            }
            // Ctrl+C — clear editor or abort.
            Key::Ctrl('c') => {
                if self.agent_running {
                    self.send_abort();
                } else {
                    self.editor.set_text("");
                }
                return;
            }
            // Escape — abort running agent.
            Key::Escape => {
                if self.agent_running {
                    self.send_abort();
                }
                return;
            }
            // Alt+Enter — submit.
            Key::Alt('\r') | Key::Alt('\n') => {
                // Handled by editor, but check for submit after.
            }
            // Page Up/Down — scroll chat.
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

        // Check if editor submitted.
        if let Some(text) = self.editor.take_submit() {
            self.handle_submit(&text);
        }
    }

    fn handle_submit(&mut self, text: &str) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }

        // Slash commands handled locally.
        match trimmed {
            "/quit" | "/exit" => {
                self.should_exit = true;
                return;
            }
            "/clear" => {
                self.send_clear_history();
                self.chat.clear();
                return;
            }
            _ => {}
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
        let _ = self.send_command(cmd);
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
            Event::TurnEnd { .. } => {
                self.chat.finalize_assistant();
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
                ..
            } => {
                if command == "get_state" && success {
                    if let Some(data) = data {
                        if let Some(model) = data.get("model").and_then(|m| m.as_str()) {
                            self.footer.set_model(model);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // ── Rendering ─────────────────────────────────────────────────────

    fn render(&mut self) {
        let width = self.terminal.width;
        let height = self.terminal.height;

        // Compose the full screen from components.
        let mut lines = Vec::new();

        // Header.
        let version = env!("CARGO_PKG_VERSION");
        lines.push(theme::dim(&format!(
            "quecto-tui v{} — Alt+Enter to send, Ctrl+D to exit",
            version
        )));
        lines.push(String::new());

        // Chat.
        let chat_lines = self.chat.render(width);
        lines.extend(chat_lines);

        // Spinner (if active).
        if let Some(spinner) = &mut self.spinner {
            let spinner_lines = spinner.render(width);
            lines.extend(spinner_lines);
        }

        // Calculate how much space editor + footer need.
        let editor_lines = self.editor.render(width);
        let footer_lines = self.footer.render(width);
        let bottom_height = editor_lines.len() + footer_lines.len();

        // Pad chat area to push editor + footer to the bottom.
        let content_height = lines.len();
        let available = height.saturating_sub(bottom_height);
        if content_height < available {
            for _ in 0..(available - content_height) {
                lines.push(String::new());
            }
        }

        // Editor.
        lines.extend(editor_lines);

        // Footer.
        lines.extend(footer_lines);

        // Truncate to terminal height.
        if lines.len() > height {
            let start = lines.len() - height;
            lines = lines[start..].to_vec();
        }

        // Write via differential renderer (use a static renderer stored in terminal).
        // For simplicity, we write directly with sync output markers.
        let mut buf = String::new();
        buf.push_str("\x1b[?2026h"); // sync start
        buf.push_str("\x1b[H"); // home cursor

        for (i, line) in lines.iter().enumerate() {
            if i > 0 {
                buf.push_str("\r\n");
            }
            buf.push_str("\x1b[2K"); // clear line
            buf.push_str(line);
        }

        // Clear remaining lines below content.
        let rendered = lines.len();
        if rendered < height {
            for _ in rendered..height {
                buf.push_str("\r\n\x1b[2K");
            }
        }

        buf.push_str("\x1b[?2026l"); // sync end

        let _ = std::io::stdout().write_all(buf.as_bytes());
        let _ = std::io::stdout().flush();
    }

    fn render_full(&mut self) {
        self.terminal.clear_screen();
        self.render();
    }

    // ── Command sending helpers ───────────────────────────────────────

    fn send_command(&mut self, cmd: Command) {
        let mut client_send = self.client.clone_sender();
        tokio::spawn(async move {
            let _ = client_send.send(&cmd).await;
        });
    }

    fn send_abort(&mut self) {
        self.send_command(Command::Abort { id: None });
    }

    fn send_clear_history(&mut self) {
        self.send_command(Command::ClearHistory { id: None });
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
