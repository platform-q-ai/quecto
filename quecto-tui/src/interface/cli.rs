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
    };
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
    let (socket, mut _child) = match flags.socket_path {
        Some(path) => (path, None),
        None => {
            // Spawn a quecto agent child process
            match spawn_agent(&flags).await {
                Ok((path, child)) => (path, Some(child)),
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
            if let Some(ref mut child) = _child {
                crate::infrastructure::process::terminate_child(
                    child,
                    crate::infrastructure::process::TERMINATE_GRACE_MS,
                )
                .await;
            }
            return 1;
        }
    };

    // Run the TUI.
    let terminal = crate::infrastructure::terminal::Terminal::new();
    let mut app = crate::interface::app::App::new(terminal, client);
    let exit_code = app.run().await;

    // Kill the child agent process group on TUI exit (catches subagents too).
    // Uses checked PID conversion to prevent u32→i32 wrapping (see #464).
    if let Some(ref mut child) = _child {
        crate::infrastructure::process::terminate_child(
            child,
            crate::infrastructure::process::TERMINATE_GRACE_MS,
        )
        .await;
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

/// Spawn a quecto agent in UDS mode and return the socket path and child handle.
///
/// The caller MUST store the child handle and call `child.kill()` + `child.wait()`
/// on TUI exit. Tokio's `Child` does NOT kill the process on drop — dropping it
/// creates an orphan. See the security review for PR #442.
async fn spawn_agent(flags: &CliFlags) -> Result<(PathBuf, tokio::process::Child), String> {
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
    let mut stderr_context = Vec::new();

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
                    &stderr_context,
                ));
            }
            Ok(Ok(_)) => {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    remember_stderr_line(&mut stderr_context, trimmed);
                }
                if let Some(path_str) = trimmed.strip_prefix(socket_prefix) {
                    let path = PathBuf::from(path_str.trim());
                    // Validate the socket path is under a safe directory.
                    if let Err(e) = validate_socket_path(&path) {
                        terminate_spawned_agent(&mut child).await;
                        return Err(e);
                    }
                    return Ok((path, child));
                }
            }
            Ok(Err(e)) => {
                terminate_spawned_agent(&mut child).await;
                return Err(format_agent_startup_failure(
                    &format!("error reading agent stderr: {e}"),
                    &stderr_context,
                ));
            }
            Err(_) => {
                terminate_spawned_agent(&mut child).await;
                return Err(format_agent_startup_failure(
                    &agent_socket_timeout_message(),
                    &stderr_context,
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

const MAX_STARTUP_STDERR_LINES: usize = 20;
const MAX_STARTUP_STDERR_LINE_CHARS: usize = 1_000;

fn remember_stderr_line(lines: &mut Vec<String>, line: &str) {
    if lines.len() == MAX_STARTUP_STDERR_LINES {
        lines.remove(0);
    }
    lines.push(truncate_stderr_line(&redact_stderr_line(line)));
}

fn truncate_stderr_line(line: &str) -> String {
    let mut truncated: String = line.chars().take(MAX_STARTUP_STDERR_LINE_CHARS).collect();
    // Short-circuit: stop scanning at the (MAX+1)th char rather than counting
    // the whole line (and re-counting the truncated copy) just to detect overflow.
    if line.chars().nth(MAX_STARTUP_STDERR_LINE_CHARS).is_some() {
        truncated.push('…');
    }
    truncated
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
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        let mut v = vec!["quecto-tui".to_string()];
        if !s.is_empty() {
            v.extend(s.split_whitespace().map(String::from));
        }
        v
    }

    #[test]
    fn parse_workflow_flags() {
        let flags = parse_flags(&args("--workflow --workflow-guards"));
        assert!(flags.workflow);
        assert!(flags.workflow_guards);
    }

    #[test]
    fn parse_no_workflow_clears_both() {
        let flags = parse_flags(&args("--workflow --workflow-guards --no-workflow"));
        assert!(!flags.workflow);
        assert!(!flags.workflow_guards);
        assert!(flags.workflow_disabled);
    }

    #[test]
    fn build_args_forward_no_workflow_to_owned_agent() {
        let flags = parse_flags(&args("--no-workflow"));
        let agent_args = build_agent_args(&flags);
        assert!(agent_args.contains(&"--no-workflow".to_string()));
        assert!(!agent_args.contains(&"--workflow".to_string()));
        assert!(!agent_args.contains(&"--workflow-guards".to_string()));
    }

    #[test]
    fn parse_config_and_system() {
        let flags = parse_flags(&args("--config ./repo/config.json --system hello"));
        assert_eq!(
            flags.config_path.unwrap().to_str().unwrap(),
            "./repo/config.json"
        );
        assert_eq!(flags.system_prompt.as_deref(), Some("hello"));
    }

    #[test]
    fn workflow_without_system_gets_default_prompt() {
        let mut flags = parse_flags(&args("--workflow"));
        apply_workflow_defaults(&mut flags);
        assert!(flags.system_prompt.is_some());
        assert!(flags.system_prompt.unwrap().contains("workflow"));
    }

    #[test]
    fn workflow_with_explicit_system_keeps_it() {
        let mut flags = parse_flags(&args("--workflow --system custom"));
        apply_workflow_defaults(&mut flags);
        assert_eq!(flags.system_prompt.as_deref(), Some("custom"));
    }

    #[test]
    fn no_workflow_no_default_prompt() {
        let mut flags = parse_flags(&args(""));
        apply_workflow_defaults(&mut flags);
        assert!(flags.system_prompt.is_none());
    }

    #[test]
    fn startup_failure_includes_agent_stderr_context() {
        let message = format_agent_startup_failure(
            "agent exited before announcing socket",
            &[
                "no LLM providers configured (set an API key or run 'quecto auth login')"
                    .to_string(),
            ],
        );

        assert!(message.contains("agent exited before announcing socket"));
        assert!(message.contains("Agent stderr:"));
        assert!(message.contains("no LLM providers configured"));
    }

    #[test]
    fn startup_failure_without_stderr_keeps_original_reason() {
        let message = format_agent_startup_failure("timeout waiting for agent socket path", &[]);
        assert_eq!(message, "timeout waiting for agent socket path");
    }

    // #808: cold-binary first launch after install must not time out at 10s.
    #[test]
    fn agent_socket_deadline_is_thirty_seconds() {
        assert_eq!(
            AGENT_SOCKET_DEADLINE,
            std::time::Duration::from_secs(30),
            "spawn->socket readiness deadline must be 30s to cover a cold-binary first launch"
        );
    }

    #[test]
    fn agent_starting_status_names_the_wait() {
        let status = agent_starting_status();
        assert!(
            status.to_lowercase().contains("starting agent"),
            "the readiness wait must surface a 'starting agent' status: {status:?}"
        );
    }

    #[test]
    fn agent_socket_timeout_message_is_actionable() {
        let message = agent_socket_timeout_message();
        // Names the cold-start / first-run-after-install cause.
        let lower = message.to_lowercase();
        assert!(
            lower.contains("cold") || lower.contains("first run") || lower.contains("first launch"),
            "timeout message must name the cold-binary / first-run cause: {message:?}"
        );
        // Names the warm remedy and the retry option.
        assert!(
            message.contains("quecto --version"),
            "timeout message must suggest running `quecto --version` to warm the binary: {message:?}"
        );
        assert!(
            lower.contains("retry") || lower.contains("try again"),
            "timeout message must mention retrying: {message:?}"
        );
    }

    #[test]
    fn agent_socket_timeout_message_flows_through_failure_formatter() {
        let message =
            format_agent_startup_failure(&agent_socket_timeout_message(), &["boom".to_string()]);
        assert!(message.contains("quecto --version"));
        assert!(message.contains("Agent stderr:"));
        assert!(message.contains("boom"));
    }

    #[test]
    fn stderr_context_redacts_common_secret_shapes() {
        let mut lines = Vec::new();
        remember_stderr_line(
            &mut lines,
            "Authorization: Bearer sk-ant-secret-token api_key=sk-test-secret-token",
        );

        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("[REDACTED]"));
        assert!(!lines[0].contains("sk-ant-secret-token"));
        assert!(!lines[0].contains("sk-test-secret-token"));
    }

    #[test]
    fn socket_path_rejects_parent_dir_components() {
        let path = PathBuf::from("/tmp/../var/run/quecto.sock");
        let err = validate_socket_path(&path).unwrap_err();
        assert!(err.contains("must not contain '..'"));
    }

    #[test]
    fn socket_path_accepts_real_socket_under_tmp() {
        let dir = std::env::temp_dir().join(format!(
            "quecto-tui-cli-test-{}-{}",
            std::process::id(),
            unique_test_suffix()
        ));
        std::fs::create_dir(&dir).expect("create temp socket dir");
        let socket = dir.join("agent.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket).expect("bind test socket");

        let result = validate_socket_path(&socket);

        drop(listener);
        let _ = std::fs::remove_file(&socket);
        let _ = std::fs::remove_dir(&dir);
        assert!(
            result.is_ok(),
            "expected socket path to validate: {result:?}"
        );
    }

    fn unique_test_suffix() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time after epoch")
            .as_nanos()
    }
}
