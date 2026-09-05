#[cfg(test)]
use std::path::Path;
use std::path::PathBuf;

pub(crate) use super::socket_path::validate_socket_path;
#[cfg(test)]
use super::socket_path::{canonical_allowed_socket_roots, canonicalize_socket_roots};

#[derive(Clone)]
pub(crate) struct CliFlags {
    pub(crate) socket_path: Option<PathBuf>,
    pub(crate) workflow: bool,
    pub(crate) workflow_guards: bool,
    pub(crate) workflow_disabled: bool,
    pub(crate) config_path: Option<PathBuf>,
    pub(crate) system_prompt: Option<String>,
    pub(crate) disable_tools: Vec<String>,
    pub(crate) persist: bool,
    pub(crate) kill_on_exit: bool,
}

pub fn run(args: Vec<String>) -> i32 {
    let mut flags = parse_flags(&args);
    apply_workflow_defaults(&mut flags);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(30)
        .enable_all()
        .build()
        .expect("failed to create tokio runtime");

    let exit_code = rt.block_on(async move { run_tui(flags).await });
    rt.shutdown_timeout(std::time::Duration::from_millis(100));
    exit_code
}

pub(crate) fn parse_flags(args: &[String]) -> CliFlags {
    let mut flags = CliFlags {
        socket_path: None,
        workflow: false,
        workflow_guards: false,
        workflow_disabled: false,
        config_path: None,
        system_prompt: None,
        disable_tools: Vec::new(),
        persist: true,
        kill_on_exit: true,
    };
    let mut system_literal_seen = false;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--socket" if i + 1 < args.len() => {
                flags.socket_path = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--config" if i + 1 < args.len() => {
                flags.config_path = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--system" if i + 1 < args.len() => {
                flags.system_prompt = Some(args[i + 1].clone());
                system_literal_seen = true;
                i += 2;
            }
            "--system-file" if i + 1 < args.len() => {
                if !system_literal_seen {
                    match std::fs::read_to_string(&args[i + 1]) {
                        Ok(content) => flags.system_prompt = Some(content),
                        Err(e) => eprintln!(
                            "quecto-tui: failed to read --system-file '{}': {e}",
                            args[i + 1]
                        ),
                    }
                }
                i += 2;
            }
            "--persist" => {
                flags.persist = true;
                i += 1;
            }
            "--no-persist" => {
                flags.persist = false;
                i += 1;
            }
            "--kill-on-exit" => {
                flags.kill_on_exit = true;
                i += 1;
            }
            "--detach-on-exit" => {
                flags.kill_on_exit = false;
                i += 1;
            }
            "--workflow" => {
                flags.workflow = true;
                flags.workflow_disabled = false;
                i += 1;
            }
            "--no-workflow" => {
                flags.workflow = false;
                flags.workflow_guards = false;
                flags.workflow_disabled = true;
                i += 1;
            }
            "--workflow-guards" => {
                flags.workflow_guards = true;
                i += 1;
            }
            "--disable-tool" => {
                if i + 1 < args.len() {
                    flags.disable_tools.push(args[i + 1].clone());
                    i += 2;
                } else {
                    eprintln!(
                        "warning: --disable-tool requires a tool name; ignoring trailing flag"
                    );
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    flags
}

fn apply_workflow_defaults(flags: &mut CliFlags) {
    if flags.workflow && flags.system_prompt.is_none() {
        flags.system_prompt = Some(
            "You are a coding assistant. Use the workflow tool to track development progress. \
             Follow the workflow steps in order and check them off as you complete them. \
             When all steps are complete, close the current issue and pick the next one."
                .to_string(),
        );
    }
}

pub(crate) use super::tab_spawn_policy::{TabSpawnPolicy, tab_spawn_flags_from_policy};

pub(crate) async fn spawn_agent_for_tab(
    flags: &CliFlags,
    pending_child_watches: crate::shell::child_watch::ChildWatchRegistry,
) -> Result<(PathBuf, crate::shell::child_watch::ChildWatch, Option<u8>), String> {
    spawn_agent_program_watched_for_tab("quecto", flags, pending_child_watches).await
}

async fn run_tui(flags: CliFlags) -> i32 {
    let tab_spawn_policy = TabSpawnPolicy::from_flags(&flags);
    let (socket, child_watch, announced_protocol) = match flags.socket_path {
        Some(path) => (path, None, None),
        None => match spawn_agent(&flags).await {
            Ok((path, child, stderr_tail, announced_protocol)) => {
                let watch = crate::shell::child_watch::watch_child(child, stderr_tail);
                (path, Some(watch), announced_protocol)
            }
            Err(e) => {
                eprintln!("Failed to start quecto agent: {e}");
                return 1;
            }
        },
    };

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = std::io::Write::write_all(
            &mut std::io::stdout(),
            b"\x1b[?1049l\x1b[?2004l\x1b[?25h\x1b[0m\x1b[>4;0m\x1b[<u",
        );
        let _ = std::io::Write::flush(&mut std::io::stdout());
        // SAFETY: termios calls operate on stdin fd 0; return values are checked before use.
        unsafe {
            let fd = 0; // stdin
            let mut termios: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(fd, &mut termios) == 0 {
                termios.c_lflag |= libc::ICANON | libc::ECHO | libc::ISIG;
                termios.c_iflag |= libc::ICRNL;
                termios.c_oflag |= libc::OPOST;
                libc::tcsetattr(fd, libc::TCSANOW, &termios);
            }
        }
        let _ = std::io::Write::write_all(&mut std::io::stdout(), b"\r\n");
        let _ = std::io::Write::flush(&mut std::io::stdout());
        default_hook(info);
    }));

    let speaks_frames = should_speak_frames(announced_protocol);
    let connect = async {
        if speaks_frames {
            crate::protocol::client::Client::connect(&socket).await
        } else {
            crate::protocol::client::Client::connect_legacy(&socket).await
        }
    };
    let client = match connect.await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to connect to agent: {e}");
            if let Some(watch) = &child_watch {
                watch.terminate().await;
            }
            return 1;
        }
    };

    let terminal = crate::shell::terminal::Terminal::new();
    let mut app = crate::shell::app::App::new(terminal, client);
    app.tab_spawn_policy = Some(tab_spawn_policy);
    app.ac_mut().socket_path = Some(socket.clone()); // AC4 durability
    if let Some(watch) = &child_watch {
        app.ac_mut().child_pid = watch.pid();
        app.set_child_exit_watch(watch.clone());
    }
    app.set_ordinary_exit_kill_owned(flags.kill_on_exit);
    app.persist_default_durability();
    let exit_code = app.run().await;

    drop(child_watch);

    exit_code
}

pub(crate) fn build_agent_args(flags: &CliFlags) -> Vec<String> {
    let mut args = vec!["agent".to_string(), "--mode".to_string(), "uds".to_string()];
    if flags.persist {
        args.push("--persist".to_string());
    }
    if flags.workflow {
        args.push("--workflow".to_string());
    }
    if flags.workflow_disabled {
        args.push("--no-workflow".to_string());
    }
    if flags.workflow_guards {
        args.push("--workflow-guards".to_string());
    }
    if let Some(ref path) = flags.config_path {
        args.push("--config".to_string());
        args.push(path.to_string_lossy().to_string());
    }
    if let Some(ref prompt) = flags.system_prompt {
        args.push("--system".to_string());
        args.push(prompt.clone());
    }
    for tool in &flags.disable_tools {
        args.push("--disable-tool".to_string());
        args.push(tool.clone());
    }
    args
}

pub const AGENT_SOCKET_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);

/// so the wait is not a silent pause (#808).
pub fn agent_starting_status() -> &'static str {
    "starting agent… (first launch after install may take a few seconds)"
}

/// Actionable message for a spawn→socket-path deadline miss (#808).
///
/// Names the likely cold-binary / first-run-after-install cause and the remedy
/// (warm the binary with `quecto --version`, then retry) instead of the bare
/// "timeout waiting for agent socket path".
pub fn agent_socket_timeout_message() -> String {
    format!(
        "timeout waiting for agent socket path after {}s. This commonly happens on \
         the first run / first launch after `cargo install`: the freshly installed \
         quecto binary is cold in the OS page cache, so the kernel can take longer \
         than usual to start. Remedy: run `quecto --version` once to warm the binary, \
         then retry — subsequent launches are fast.",
        AGENT_SOCKET_DEADLINE.as_secs()
    )
}

/// Spawn a quecto agent in UDS mode and return the socket path, the child
/// handle, and the ring buffer its post-startup stderr keeps draining into
/// (#1047 — so a later panic-abort message is captured, not lost).
///
/// The caller MUST store the child handle and call `child.kill()` + `child.wait()`
/// on TUI exit. Tokio's `Child` does NOT kill the process on drop — dropping it
/// creates an orphan. See the security review for PR #442.
pub(crate) async fn spawn_agent(
    flags: &CliFlags,
) -> Result<
    (
        PathBuf,
        tokio::process::Child,
        crate::shell::child_watch::StderrTail,
        Option<u8>,
    ),
    String,
> {
    spawn_agent_program("quecto", flags).await
}

pub(crate) async fn spawn_agent_program_watched_for_tab(
    program: &str,
    flags: &CliFlags,
    pending_child_watches: crate::shell::child_watch::ChildWatchRegistry,
) -> Result<(PathBuf, crate::shell::child_watch::ChildWatch, Option<u8>), String> {
    use tokio::io::AsyncBufReadExt;
    let args = build_agent_args(flags);
    let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut child = tokio::process::Command::new(program)
        .args(&args_ref)
        .stderr(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stdin(std::process::Stdio::null())
        // Create a new process group so ordinary exit can kill the tab agent
        // and any TUI-owned descendants without touching unrelated agents.
        .process_group(0)
        .spawn()
        .map_err(|e| format!("failed to spawn {program}: {e}"))?;

    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "failed to capture agent stderr".to_string())?;
    let stderr_context = crate::shell::child_watch::StderrTail::default();
    let watch = crate::shell::child_watch::watch_child(child, stderr_context.clone());
    if let Ok(mut pending) = pending_child_watches.lock() {
        pending.push(watch.clone());
    }

    let mut reader = tokio::io::BufReader::new(stderr);
    let mut line = String::new();
    let socket_prefix = "quecto-agent-socket: ";
    let protocol_prefix = quecto_line_io::PROTOCOL_ANNOUNCE_PREFIX;
    let mut announced_protocol: Option<u8> = None;
    let deadline = tokio::time::Instant::now() + AGENT_SOCKET_DEADLINE;

    eprintln!("{}", agent_starting_status());

    loop {
        line.clear();
        let read_future = reader.read_line(&mut line);
        let result = tokio::time::timeout_at(deadline, read_future).await;

        match result {
            Ok(Ok(0)) => {
                watch.terminate().await;
                return Err(format_agent_startup_failure(
                    "agent exited before announcing socket",
                    &stderr_context.lines(),
                ));
            }
            Ok(Ok(_)) => {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    remember_stderr_line(&stderr_context, trimmed);
                }
                if let Some(version) = trimmed.strip_prefix(protocol_prefix) {
                    announced_protocol = version.trim().parse().ok();
                }
                if let Some(path_str) = trimmed.strip_prefix(socket_prefix) {
                    let path = PathBuf::from(path_str.trim());
                    if let Err(e) = validate_socket_path(&path) {
                        watch.terminate().await;
                        return Err(e);
                    }
                    spawn_stderr_drain(reader, stderr_context.clone());
                    return Ok((path, watch, announced_protocol));
                }
            }
            Ok(Err(e)) => {
                watch.terminate().await;
                return Err(format_agent_startup_failure(
                    &format!("error reading agent stderr: {e}"),
                    &stderr_context.lines(),
                ));
            }
            Err(_) => {
                watch.terminate().await;
                return Err(format_agent_startup_failure(
                    &agent_socket_timeout_message(),
                    &stderr_context.lines(),
                ));
            }
        }
    }
}

/// [`spawn_agent`] with the agent binary injectable, so tests can drive the
/// REAL spawn path (socket announcement parsing, post-startup stderr drain
/// wiring) with a stand-in script — reverting the drain hookup must fail a
/// test, not just the manually-wired drain unit tests (#1051 final review).
/// Whether the TUI should speak length-prefixed frames to an agent that
/// announced protocol version `announced` in its socket announcement. Frames
/// for v2+ ([`quecto_line_io::PROTOCOL_VERSION`]); legacy NDJSON for a
/// pre-#1059 agent that announced nothing (deprecation window, ADR-0008).
fn should_speak_frames(announced: Option<u8>) -> bool {
    announced.is_some_and(|v| v >= quecto_line_io::PROTOCOL_VERSION)
}

async fn spawn_agent_program(
    program: &str,
    flags: &CliFlags,
) -> Result<
    (
        PathBuf,
        tokio::process::Child,
        crate::shell::child_watch::StderrTail,
        // Protocol version from the `quecto-agent-protocol: N` announcement
        // line, `None` for pre-#1059 agents (legacy NDJSON framing).
        Option<u8>,
    ),
    String,
> {
    use tokio::io::AsyncBufReadExt;
    let args = build_agent_args(flags);
    let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut child = tokio::process::Command::new(program)
        .args(&args_ref)
        .stderr(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stdin(std::process::Stdio::null())
        // Create a new process group so we can kill the agent + all its subagents.
        .process_group(0)
        .spawn()
        .map_err(|e| format!("failed to spawn {program}: {e}"))?;

    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "failed to capture agent stderr".to_string())?;

    let mut reader = tokio::io::BufReader::new(stderr);
    let mut line = String::new();
    let stderr_context = crate::shell::child_watch::StderrTail::default();

    // Read stderr lines looking for the socket path announcement (and the
    // protocol-version line that precedes it since #1059).
    let socket_prefix = "quecto-agent-socket: ";
    // Shared single source of truth (quecto-line-io), so the producer (agent
    // announcement) and this consumer can never drift into a silent mismatch.
    let protocol_prefix = quecto_line_io::PROTOCOL_ANNOUNCE_PREFIX;
    let mut announced_protocol: Option<u8> = None;
    let deadline = tokio::time::Instant::now() + AGENT_SOCKET_DEADLINE;

    // Surface a brief status so the readiness wait is not a silent pause (#808).
    eprintln!("{}", agent_starting_status());

    loop {
        line.clear();
        let read_future = reader.read_line(&mut line);
        let result = tokio::time::timeout_at(deadline, read_future).await;

        match result {
            Ok(Ok(0)) => {
                terminate_spawned_agent(&mut child).await;
                return Err(format_agent_startup_failure(
                    "agent exited before announcing socket",
                    &stderr_context.lines(),
                ));
            }
            Ok(Ok(_)) => {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    remember_stderr_line(&stderr_context, trimmed);
                }
                if let Some(version) = trimmed.strip_prefix(protocol_prefix) {
                    announced_protocol = version.trim().parse().ok();
                }
                if let Some(path_str) = trimmed.strip_prefix(socket_prefix) {
                    let path = PathBuf::from(path_str.trim());
                    // Validate the socket path is under a safe directory.
                    if let Err(e) = validate_socket_path(&path) {
                        terminate_spawned_agent(&mut child).await;
                        return Err(e);
                    }
                    // Keep draining stderr AFTER startup (#1047): under the
                    // workspace `panic = "abort"` a mid-session agent panic
                    // writes its message here and then kills the process, so
                    // dropping the reader would make the abort undiagnosable.
                    spawn_stderr_drain(reader, stderr_context.clone());
                    return Ok((path, child, stderr_context, announced_protocol));
                }
            }
            Ok(Err(e)) => {
                terminate_spawned_agent(&mut child).await;
                return Err(format_agent_startup_failure(
                    &format!("error reading agent stderr: {e}"),
                    &stderr_context.lines(),
                ));
            }
            Err(_) => {
                terminate_spawned_agent(&mut child).await;
                return Err(format_agent_startup_failure(
                    &agent_socket_timeout_message(),
                    &stderr_context.lines(),
                ));
            }
        }
    }
}

async fn terminate_spawned_agent(child: &mut tokio::process::Child) {
    crate::shell::process::terminate_child(child, crate::shell::process::TERMINATE_GRACE_MS).await;
}

const MAX_STARTUP_STDERR_LINE_CHARS: usize = 1_000;

/// Byte cap on a single drained stderr line (#1051 review pattern: no uncapped
/// reads) — a child spewing an endless newline-free stream must not grow the
/// line buffer without bound. Anything past the cap is discarded.
const MAX_DRAIN_STDERR_LINE_BYTES: usize = 8 * 1024;

fn remember_stderr_line(tail: &crate::shell::child_watch::StderrTail, line: &str) {
    tail.push(truncate_stderr_line(&redact_stderr_line(line)));
}

/// Keep draining the agent child's stderr AFTER startup into the shared ring
/// buffer, so a mid-session panic-abort message survives the process (#1047).
/// Lines are redacted and truncated exactly like the startup capture. The task
/// ends when the child closes its stderr (i.e. exits) and marks the tail
/// drained, so the disconnect path can wait for the buffered panic message
/// instead of racing this task (#1051 final review).
pub fn spawn_stderr_drain<R>(mut reader: R, tail: crate::shell::child_watch::StderrTail)
where
    R: tokio::io::AsyncBufRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut line = String::new();
        loop {
            match read_stderr_line_capped(&mut reader, &mut line).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        remember_stderr_line(&tail, trimmed);
                    }
                }
            }
        }
        tail.mark_drained();
    });
}

/// Read one `\n`-terminated line, keeping at most [`MAX_DRAIN_STDERR_LINE_BYTES`]
/// of it (the rest of an oversized line is consumed and discarded). Returns the
/// number of bytes consumed from the stream; 0 means EOF.
async fn read_stderr_line_capped<R>(reader: &mut R, out: &mut String) -> std::io::Result<usize>
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    use tokio::io::AsyncBufReadExt;
    out.clear();
    let mut bytes: Vec<u8> = Vec::new();
    let mut consumed_total = 0usize;
    loop {
        let buf = reader.fill_buf().await?;
        if buf.is_empty() {
            break; // EOF (possibly mid-line)
        }
        let (chunk, consumed, line_done) = match buf.iter().position(|&b| b == b'\n') {
            Some(i) => (&buf[..i], i + 1, true),
            None => (buf, buf.len(), false),
        };
        if bytes.len() < MAX_DRAIN_STDERR_LINE_BYTES {
            let take = chunk.len().min(MAX_DRAIN_STDERR_LINE_BYTES - bytes.len());
            bytes.extend_from_slice(&chunk[..take]);
        }
        reader.consume(consumed);
        consumed_total += consumed;
        if line_done {
            break;
        }
    }
    // Decode ONCE per line (#1051 final review): a multi-byte character split
    // across pipe reads must not be mangled into U+FFFD by per-chunk decoding.
    // A character sliced by the byte cap itself is dropped, not replaced.
    match std::str::from_utf8(&bytes) {
        Ok(s) => out.push_str(s),
        // `error_len() == None`: the only problem is an incomplete sequence
        // at the very end (cap or EOF mid-character) — keep the valid prefix.
        Err(e) if e.error_len().is_none() => {
            out.push_str(&String::from_utf8_lossy(&bytes[..e.valid_up_to()]));
        }
        // Genuinely invalid UTF-8 mid-line (binary output): lossy-decode.
        Err(_) => out.push_str(&String::from_utf8_lossy(&bytes)),
    }
    Ok(consumed_total)
}

fn truncate_stderr_line(line: &str) -> String {
    crate::components::utils::truncate_chars_with_ellipsis(line, MAX_STARTUP_STDERR_LINE_CHARS, "…")
}

fn redact_stderr_line(line: &str) -> String {
    let mut redacted = redact_named_secret_values(line);
    redacted = redact_bearer_tokens(&redacted);
    redact_secret_tokens(&redacted)
}

fn redact_named_secret_values(line: &str) -> String {
    let names = [
        "api_key",
        "apikey",
        "apiKey",
        "access_token",
        "refresh_token",
        "id_token",
        "authorization",
        "OPENAI_API_KEY",
        "ANTHROPIC_API_KEY",
        "GOOGLE_API_KEY",
    ];
    let mut out = line.to_string();
    for name in names {
        out = redact_value_after_name(&out, name);
    }
    out
}

fn redact_value_after_name(input: &str, name: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let lower = input.to_ascii_lowercase();
    let needle = name.to_ascii_lowercase();
    let mut cursor = 0;

    while let Some(rel) = lower[cursor..].find(&needle) {
        let start = cursor + rel;
        let after_name = start + needle.len();
        let Some(sep_rel) = input[after_name..].find([':', '=']) else {
            break;
        };
        let sep = after_name + sep_rel;
        out.push_str(&input[cursor..=sep]);
        let mut value_start = sep + 1;
        while input[value_start..].starts_with(char::is_whitespace) {
            out.push(input[value_start..].chars().next().unwrap());
            value_start += input[value_start..].chars().next().unwrap().len_utf8();
        }
        let quote = input[value_start..]
            .chars()
            .next()
            .filter(|c| *c == '"' || *c == '\'');
        if let Some(q) = quote {
            out.push(q);
            value_start += q.len_utf8();
        }
        out.push_str("[REDACTED]");
        let mut value_end = value_start;
        for (idx, ch) in input[value_start..].char_indices() {
            let end = value_start + idx;
            let stop = if quote.is_some() {
                Some(ch) == quote
            } else {
                ch.is_whitespace() || ch == ',' || ch == '}'
            };
            if stop {
                value_end = end;
                break;
            }
            value_end = end + ch.len_utf8();
        }
        if let Some(q) = quote {
            if input[value_end..].starts_with(q) {
                out.push(q);
                value_end += q.len_utf8();
            }
        }
        cursor = value_end;
    }
    out.push_str(&input[cursor..]);
    out
}

fn redact_bearer_tokens(input: &str) -> String {
    input
        .split_whitespace()
        .scan(false, |previous_was_bearer, token| {
            let current = if *previous_was_bearer {
                "[REDACTED]".to_string()
            } else {
                token.to_string()
            };
            *previous_was_bearer = token.eq_ignore_ascii_case("bearer");
            Some(current)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn redact_secret_tokens(input: &str) -> String {
    input
        .split_whitespace()
        .map(|token| {
            let trimmed =
                token.trim_matches(|c: char| c == '"' || c == '\'' || c == ',' || c == ';');
            if looks_like_secret_token(trimmed) {
                token.replace(trimmed, "[REDACTED]")
            } else {
                token.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn looks_like_secret_token(token: &str) -> bool {
    let prefixes = [
        "sk-",
        "sk_ant_",
        "sk-ant-",
        "AIza",
        "ghp_",
        "github_pat_",
        "xoxb-",
    ];
    token.len() >= 16 && prefixes.iter().any(|prefix| token.starts_with(prefix))
}

fn format_agent_startup_failure(reason: &str, stderr_lines: &[String]) -> String {
    if stderr_lines.is_empty() {
        return reason.to_string();
    }

    format!("{}\nAgent stderr:\n{}", reason, stderr_lines.join("\n"))
}

#[cfg(test)]
#[path = "cli_cov_tests.rs"]
mod cli_cov_tests;

#[cfg(test)]
#[path = "cli_tab_spawn_tests.rs"]
mod cli_tab_spawn_tests;
#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
