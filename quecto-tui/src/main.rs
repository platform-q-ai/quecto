//! quecto-tui — Lightweight terminal UI client for quecto.
//!
//! Spawns (or connects to) a `quecto agent --mode uds` process and provides
//! a rich interactive terminal interface over the UDS JSON-lines protocol.

use std::path::PathBuf;

/// Parsed CLI flags for quecto-tui.
struct CliFlags {
    socket_path: Option<PathBuf>,
    no_sandbox: bool,
    network: bool,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let flags = parse_flags(&args);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to create tokio runtime");

    rt.block_on(async move {
        let result = run(flags).await;
        std::process::exit(result);
    });
}

/// Parse CLI flags from command-line arguments.
fn parse_flags(args: &[String]) -> CliFlags {
    let mut flags = CliFlags {
        socket_path: None,
        no_sandbox: false,
        network: false,
    };
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--socket" if i + 1 < args.len() => {
                flags.socket_path = Some(PathBuf::from(&args[i + 1]));
                i += 2;
            }
            "--no-sandbox" => {
                flags.no_sandbox = true;
                i += 1;
            }
            "--network" => {
                flags.network = true;
                i += 1;
            }
            _ => i += 1,
        }
    }
    flags
}

/// Main async entry point.
async fn run(flags: CliFlags) -> i32 {
    let (socket, mut _child) = match flags.socket_path {
        Some(path) => (path, None),
        None => {
            // Spawn a quecto agent child process
            match spawn_agent(flags.no_sandbox, flags.network).await {
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
    let client = match quecto_tui::client::Client::connect(&socket).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to connect to agent: {e}");
            if let Some(ref mut child) = _child {
                quecto_tui::process::terminate_child(
                    child,
                    quecto_tui::process::TERMINATE_GRACE_MS,
                )
                .await;
            }
            return 1;
        }
    };

    // Run the TUI.
    let terminal = quecto_tui::terminal::Terminal::new();
    let mut app = quecto_tui::app::App::new(terminal, client);
    let exit_code = app.run().await;

    // Kill the child agent process group on TUI exit (catches subagents too).
    // Uses checked PID conversion to prevent u32→i32 wrapping (see #464).
    if let Some(ref mut child) = _child {
        quecto_tui::process::terminate_child(child, quecto_tui::process::TERMINATE_GRACE_MS).await;
    }

    exit_code
}

/// Spawn a quecto agent in UDS mode and return the socket path and child handle.
///
/// The caller MUST store the child handle and call `child.kill()` + `child.wait()`
/// on TUI exit. Tokio's `Child` does NOT kill the process on drop — dropping it
/// creates an orphan. See the security review for PR #442.
async fn spawn_agent(
    no_sandbox: bool,
    network: bool,
) -> Result<(PathBuf, tokio::process::Child), String> {
    use tokio::io::AsyncBufReadExt;
    use tokio::process::Command;

    let mut args = vec!["agent", "--mode", "uds"];
    if no_sandbox {
        args.push("--no-sandbox");
    }
    if network {
        args.push("--network");
    }

    let mut child = Command::new("quecto")
        .args(&args)
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

    // Read stderr lines looking for the socket path announcement
    let socket_prefix = "quecto-agent-socket: ";
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(10);

    loop {
        line.clear();
        let read_future = reader.read_line(&mut line);
        let result = tokio::time::timeout_at(deadline, read_future).await;

        match result {
            Ok(Ok(0)) => {
                let _ = child.kill().await;
                return Err("agent exited before announcing socket".to_string());
            }
            Ok(Ok(_)) => {
                let trimmed = line.trim();
                if let Some(path_str) = trimmed.strip_prefix(socket_prefix) {
                    let path = PathBuf::from(path_str.trim());
                    // Validate the socket path is under a safe directory
                    validate_socket_path(&path)?;
                    return Ok((path, child));
                }
            }
            Ok(Err(e)) => {
                let _ = child.kill().await;
                return Err(format!("error reading agent stderr: {e}"));
            }
            Err(_) => {
                let _ = child.kill().await;
                return Err("timeout waiting for agent socket path".to_string());
            }
        }
    }
}

/// Validate that a socket path is under a safe, expected directory.
///
/// Accepts paths under /tmp, $TMPDIR, $XDG_RUNTIME_DIR, or the user's home.
/// Rejects absolute paths under system directories to prevent the TUI from
/// connecting to arbitrary sockets if the agent binary is compromised.
fn validate_socket_path(path: &std::path::Path) -> Result<(), String> {
    let path_str = path.to_string_lossy();

    // Must be absolute
    if !path.is_absolute() {
        return Err(format!("socket path is not absolute: {path_str}"));
    }

    // Allow /tmp, /run/user/*, or any path under user's home
    let allowed_prefixes: Vec<PathBuf> = {
        let mut v = vec![PathBuf::from("/tmp")];
        if let Ok(tmpdir) = std::env::var("TMPDIR") {
            v.push(PathBuf::from(tmpdir));
        }
        if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
            v.push(PathBuf::from(xdg));
        }
        if let Some(home) = std::env::var_os("HOME") {
            v.push(PathBuf::from(home));
        }
        v
    };

    for prefix in &allowed_prefixes {
        if path.starts_with(prefix) {
            return Ok(());
        }
    }

    Err(format!(
        "socket path '{}' is not under an expected directory (/tmp, $TMPDIR, $XDG_RUNTIME_DIR, $HOME)",
        path_str
    ))
}
