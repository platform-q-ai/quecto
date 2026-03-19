//! quecto-tui — Lightweight terminal UI client for quecto.
//!
//! Spawns (or connects to) a `quecto agent --mode uds` process and provides
//! a rich interactive terminal interface over the UDS JSON-lines protocol.

use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    let socket_path = parse_socket_arg(&args);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("failed to create tokio runtime");

    rt.block_on(async move {
        let result = run(socket_path).await;
        std::process::exit(result);
    });
}

/// Parse --socket <path> from command-line arguments.
fn parse_socket_arg(args: &[String]) -> Option<PathBuf> {
    let mut i = 1;
    while i < args.len() {
        if args[i] == "--socket" && i + 1 < args.len() {
            return Some(PathBuf::from(&args[i + 1]));
        }
        i += 1;
    }
    None
}

/// Main async entry point.
async fn run(socket_path: Option<PathBuf>) -> i32 {
    let socket = match socket_path {
        Some(path) => path,
        None => {
            // Spawn a quecto agent child process
            match spawn_agent().await {
                Ok(path) => path,
                Err(e) => {
                    eprintln!("Failed to start quecto agent: {e}");
                    return 1;
                }
            }
        }
    };

    eprintln!("Connecting to: {}", socket.display());

    // Connect to the agent
    let mut client = match quecto_tui::client::Client::connect(&socket).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to connect to agent: {e}");
            return 1;
        }
    };

    // Send a get_state to verify connectivity
    let cmd = quecto_tui::client::Command::GetState {
        id: Some("init".into()),
    };
    if let Err(e) = client.send(&cmd).await {
        eprintln!("Failed to send command: {e}");
        return 1;
    }

    // Read events until connection closes
    while let Some(event) = client.recv().await {
        match event {
            quecto_tui::client::Event::Response {
                command, success, ..
            } => {
                eprintln!("Response: command={command}, success={success}");
            }
            quecto_tui::client::Event::Token { token } => {
                eprint!("{token}");
            }
            quecto_tui::client::Event::AgentStart => {
                eprintln!("[agent started]");
            }
            quecto_tui::client::Event::AgentEnd { .. } => {
                eprintln!("\n[agent done]");
            }
            _ => {}
        }
    }

    0
}

/// Spawn a quecto agent in UDS mode and return the socket path.
async fn spawn_agent() -> Result<PathBuf, String> {
    use tokio::io::AsyncBufReadExt;
    use tokio::process::Command;

    let mut child = Command::new("quecto")
        .args(["agent", "--mode", "uds", "--no-sandbox", "--network"])
        .stderr(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stdin(std::process::Stdio::null())
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
            Ok(Ok(0)) => return Err("agent exited before announcing socket".to_string()),
            Ok(Ok(_)) => {
                let trimmed = line.trim();
                if let Some(path) = trimmed.strip_prefix(socket_prefix) {
                    return Ok(PathBuf::from(path.trim()));
                }
            }
            Ok(Err(e)) => return Err(format!("error reading agent stderr: {e}")),
            Err(_) => return Err("timeout waiting for agent socket path".to_string()),
        }
    }
}
