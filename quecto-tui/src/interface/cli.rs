//! quecto-tui — Lightweight terminal UI client for quecto.
//!
//! Spawns (or connects to) a `quecto agent --mode uds` process and provides
//! a rich interactive terminal interface over the UDS JSON-lines protocol.

use std::os::unix::fs::FileTypeExt;
use std::path::{Component, Path, PathBuf};

/// Parsed CLI flags for quecto-tui.
struct CliFlags {
    socket_path: Option<PathBuf>,
    no_sandbox: bool,
    workflow: bool,
    workflow_guards: bool,
    workflow_disabled: bool,
    config_path: Option<PathBuf>,
    system_prompt: Option<String>,
    /// Tool names to forward to the spawned coordinator as `--disable-tool <name>`
    /// (repeatable). Empty means none are disabled (#957 TUI forward fix).
    disable_tools: Vec<String>,
}

pub fn run(args: Vec<String>) -> i32 {
    let mut flags = parse_flags(&args);
    apply_workflow_defaults(&mut flags);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to create tokio runtime");

    let exit_code = rt.block_on(async move { run_tui(flags).await });
    rt.shutdown_timeout(std::time::Duration::from_millis(100));
    exit_code
}

/// Parse CLI flags from command-line arguments.
fn parse_flags(args: &[String]) -> CliFlags {
    let mut flags = CliFlags {
        socket_path: None,
        no_sandbox: false,
        workflow: false,
        workflow_guards: false,
        workflow_disabled: false,
        config_path: None,
        system_prompt: None,
        disable_tools: Vec::new(),
    };
    // An explicit `--system` literal takes precedence over `--system-file`
    // regardless of order; track it so a later/earlier `--system-file` can't
    // clobber an operator-supplied literal.
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
            // `--system-file <path>` reads the file's contents as the system
            // prompt (handy for long, evolving prompts). An explicit `--system`
            // literal wins, in either order. A read error is reported and the
            // prompt is left unset (falling back to defaults) so it's visible.
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
            "--no-sandbox" => {
                flags.no_sandbox = true;
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
            // `--disable-tool <name>` (repeatable) removes a tool from the
            // spawned coordinator's registry. Forwarded verbatim to the child so
            // the operator can launch it read-only (e.g. write/edit off) (#957).
            "--disable-tool" => {
                if i + 1 < args.len() {
                    flags.disable_tools.push(args[i + 1].clone());
                    i += 2;
                } else {
                    // A trailing `--disable-tool` with no value would silently make a
                    // read-only launch read-write; surface it instead of dropping it.
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

/// Apply sensible defaults when workflow is enabled.
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

/// Main async entry point.
async fn run_tui(flags: CliFlags) -> i32 {
    // For a TUI-owned child, the #1047 exit watcher takes ownership of the
    // `Child` (so it can reap it and record an exit diagnosis for the
    // disconnect notification). Termination also goes through the watcher —
    // never by a stored raw PID, which could be recycled after the watcher
    // reaps the child (#1051 review: PID-reuse race).
    let (socket, child_watch) = match flags.socket_path {
        Some(path) => (path, None),
        None => {
            // Spawn a quecto agent child process
            match spawn_agent(&flags).await {
                Ok((path, child, stderr_tail)) => {
                    let watch = crate::infrastructure::child_watch::watch_child(child, stderr_tail);
                    (path, Some(watch))
                }
                Err(e) => {
                    eprintln!("Failed to start quecto agent: {e}");
                    return 1;
                }
            }
        }
    };

    // Install panic handler to restore terminal before printing panic.
    // We restore termios via libc directly — the Terminal struct may not be
    // accessible from the panic hook.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Leave alt screen first, then reset modes on the main buffer.
        let _ = std::io::Write::write_all(
            &mut std::io::stdout(),
            b"\x1b[?1049l\x1b[?2004l\x1b[?25h\x1b[0m\x1b[>4;0m\x1b[<u",
        );
        let _ = std::io::Write::flush(&mut std::io::stdout());
        // Restore termios to cooked mode (best-effort).
        // SAFETY: termios calls operate on stdin fd 0; return values are checked before using the struct.
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

    // Connect to the agent.
    let client = match crate::infrastructure::client::Client::connect(&socket).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to connect to agent: {e}");
            if let Some(watch) = &child_watch {
                watch.terminate().await;
            }
            return 1;
        }
    };

    // Run the TUI.
    let terminal = crate::infrastructure::terminal::Terminal::new();
    let mut app = crate::interface::app::App::new(terminal, client);
    if let Some(watch) = &child_watch {
        app.set_child_exit_watch(watch.clone());
    }
    let exit_code = app.run().await;

    // Kill the child agent process group on TUI exit (catches subagents too),
    // via the watcher so an already-reaped (possibly recycled) PID is never
    // signalled (#1051 review).
    if let Some(watch) = &child_watch {
        watch.terminate().await;
    }

    exit_code
}

/// Build the `quecto agent` argv used for an owned TUI child process.
fn build_agent_args(flags: &CliFlags) -> Vec<String> {
    let mut args = vec!["agent".to_string(), "--mode".to_string(), "uds".to_string()];
    if flags.no_sandbox {
        args.push("--no-sandbox".to_string());
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

/// Spawn→socket-path readiness deadline.
///
/// 30s comfortably covers a cold-binary first launch after `cargo install`
/// (paging the freshly-written kernel binary into the OS page cache + first-run
/// config/credential load) without letting a genuinely-hung kernel hang too
/// long. See #808. `scripts/run-tui.sh` pre-warms the binary so this window is
/// only load-bearing for direct `quecto-tui …` invocations.
pub const AGENT_SOCKET_DEADLINE: std::time::Duration = std::time::Duration::from_secs(30);

/// Status line shown while waiting for the spawned agent to announce its socket,
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
async fn spawn_agent(
    flags: &CliFlags,
) -> Result<
    (
        PathBuf,
        tokio::process::Child,
        crate::infrastructure::child_watch::StderrTail,
    ),
    String,
> {
    use tokio::io::AsyncBufReadExt;
    use tokio::process::Command;

    let args = build_agent_args(flags);
    let args_ref: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut child = Command::new("quecto")
        .args(&args_ref)
        .stderr(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stdin(std::process::Stdio::null())
        // Create a new process group so we can kill the agent + all its subagents.
        .process_group(0)
        .spawn()
        .map_err(|e| format!("failed to spawn quecto: {e}"))?;

    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "failed to capture agent stderr".to_string())?;

    let mut reader = tokio::io::BufReader::new(stderr);
    let mut line = String::new();
    let stderr_context = crate::infrastructure::child_watch::StderrTail::default();

    // Read stderr lines looking for the socket path announcement
    let socket_prefix = "quecto-agent-socket: ";
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
                    return Ok((path, child, stderr_context));
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
    crate::infrastructure::process::terminate_child(
        child,
        crate::infrastructure::process::TERMINATE_GRACE_MS,
    )
    .await;
}

const MAX_STARTUP_STDERR_LINE_CHARS: usize = 1_000;

/// Byte cap on a single drained stderr line (#1051 review pattern: no uncapped
/// reads) — a child spewing an endless newline-free stream must not grow the
/// line buffer without bound. Anything past the cap is discarded.
const MAX_DRAIN_STDERR_LINE_BYTES: usize = 8 * 1024;

fn remember_stderr_line(tail: &crate::infrastructure::child_watch::StderrTail, line: &str) {
    tail.push(truncate_stderr_line(&redact_stderr_line(line)));
}

/// Keep draining the agent child's stderr AFTER startup into the shared ring
/// buffer, so a mid-session panic-abort message survives the process (#1047).
/// Lines are redacted and truncated exactly like the startup capture. The task
/// ends when the child closes its stderr (i.e. exits).
pub fn spawn_stderr_drain<R>(mut reader: R, tail: crate::infrastructure::child_watch::StderrTail)
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
    let mut consumed_total = 0usize;
    loop {
        let buf = reader.fill_buf().await?;
        if buf.is_empty() {
            return Ok(consumed_total); // EOF (possibly mid-line)
        }
        let (chunk, consumed, line_done) = match buf.iter().position(|&b| b == b'\n') {
            Some(i) => (&buf[..i], i + 1, true),
            None => (buf, buf.len(), false),
        };
        if out.len() < MAX_DRAIN_STDERR_LINE_BYTES {
            let take = chunk.len().min(MAX_DRAIN_STDERR_LINE_BYTES - out.len());
            out.push_str(&String::from_utf8_lossy(&chunk[..take]));
        }
        reader.consume(consumed);
        consumed_total += consumed;
        if line_done {
            return Ok(consumed_total);
        }
    }
}

fn truncate_stderr_line(line: &str) -> String {
    crate::interface::utils::truncate_chars_with_ellipsis(line, MAX_STARTUP_STDERR_LINE_CHARS, "…")
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

/// Validate that a socket path is under a safe, expected directory.
///
/// Accepts paths under /tmp, $TMPDIR, $XDG_RUNTIME_DIR, or the user's home.
/// Rejects absolute paths under system directories to prevent the TUI from
/// connecting to arbitrary sockets if the agent binary is compromised.
fn validate_socket_path(path: &Path) -> Result<(), String> {
    let path_str = path.to_string_lossy();

    if !path.is_absolute() {
        return Err(format!("socket path is not absolute: {path_str}"));
    }
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(format!("socket path must not contain '..': {path_str}"));
    }

    let metadata = std::fs::symlink_metadata(path)
        .map_err(|e| format!("socket path '{}' is not accessible: {e}", path_str))?;
    if metadata.file_type().is_symlink() {
        return Err(format!("socket path must not be a symlink: {path_str}"));
    }
    if !metadata.file_type().is_socket() {
        return Err(format!("socket path is not a Unix socket: {path_str}"));
    }

    let parent = path
        .parent()
        .ok_or_else(|| format!("socket path has no parent directory: {path_str}"))?;
    let canonical_parent = std::fs::canonicalize(parent).map_err(|e| {
        format!(
            "socket parent '{}' is not accessible: {e}",
            parent.display()
        )
    })?;
    let allowed_roots = canonical_allowed_socket_roots();

    if allowed_roots
        .iter()
        .any(|prefix| canonical_parent.starts_with(prefix))
    {
        return Ok(());
    }

    Err(format!(
        "socket path '{}' is not under an expected directory (/tmp, $TMPDIR, $XDG_RUNTIME_DIR, $HOME)",
        path_str
    ))
}

fn canonical_allowed_socket_roots() -> Vec<PathBuf> {
    let mut roots = vec![PathBuf::from("/tmp")];
    if let Ok(tmpdir) = std::env::var("TMPDIR") {
        roots.push(PathBuf::from(tmpdir));
    }
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        roots.push(PathBuf::from(xdg));
    }
    if let Some(home) = std::env::var_os("HOME") {
        roots.push(PathBuf::from(home));
    }
    canonicalize_socket_roots(roots)
}

/// Keep only absolute, canonicalizable roots. Split out from
/// [`canonical_allowed_socket_roots`] so the relative-path rejection can be
/// tested without mutating the process environment (which races with other
/// tests that read `TMPDIR`/`XDG_RUNTIME_DIR` under parallel execution).
fn canonicalize_socket_roots(roots: Vec<PathBuf>) -> Vec<PathBuf> {
    roots
        .into_iter()
        .filter(|root| root.is_absolute())
        .filter_map(|root| std::fs::canonicalize(root).ok())
        .collect()
}

#[cfg(test)]
#[path = "cli_cov_tests.rs"]
mod cli_cov_tests;

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
