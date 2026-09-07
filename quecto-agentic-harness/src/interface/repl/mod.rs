use std::io::{BufRead, Write};

/// Run the configuration-only interactive shell.
///
/// Command execution is supplied by the CLI adapter so this interface never
/// constructs an agent, provider, tool runtime, or persistence service.
pub fn run_repl<R, W, F>(mut reader: R, mut writer: W, is_tty: bool, mut execute: F) -> i32
where
    R: BufRead,
    W: Write,
    F: FnMut(Vec<String>, &mut R) -> (String, String, i32),
{
    if is_tty {
        let _ = writeln!(
            writer,
            "quecto v{} — Setup & Configuration",
            env!("CARGO_PKG_VERSION")
        );
        let _ = writeln!(writer, "Type help for commands, exit to quit\n");
    }

    let mut line = String::new();
    let mut exit_code = 0;
    loop {
        if is_tty {
            let _ = write!(writer, "> ");
            let _ = writer.flush();
        }
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(error) => {
                let _ = writeln!(writer, "Error reading input: {error}");
                return 1;
            }
        }
        let input = line.trim();
        if input.is_empty() {
            continue;
        }
        let command = input.strip_prefix('/').unwrap_or(input);
        if matches!(command, "exit" | "quit") {
            break;
        }
        if matches!(command, "help" | "?") {
            print_help(&mut writer);
            continue;
        }
        let args: Vec<String> = command.split_whitespace().map(str::to_owned).collect();
        if !matches!(
            args.first().map(String::as_str),
            Some("auth" | "status" | "models")
        ) {
            let _ = writeln!(
                writer,
                "Unsupported REPL command. This interface is only for login, setup, and configuration; use `quecto agent` or `quecto-tui` to operate agents."
            );
            continue;
        }
        let (stdout, stderr, command_code) = execute(args, &mut reader);
        let _ = write!(writer, "{stdout}");
        let _ = write!(writer, "{stderr}");
        if command_code != 0 {
            exit_code = command_code;
        }
    }
    exit_code
}

fn print_help(writer: &mut impl Write) {
    let _ = writeln!(writer, "Setup and configuration commands:");
    let _ = writeln!(writer, "  auth login ...   Log in to a provider");
    let _ = writeln!(writer, "  auth logout ...  Remove stored credentials");
    let _ = writeln!(writer, "  auth status      Show authentication status");
    let _ = writeln!(writer, "  status           Show effective configuration");
    let _ = writeln!(
        writer,
        "  models discover <provider-key>  Discover a provider's models"
    );
    let _ = writeln!(writer, "  help             Show this help");
    let _ = writeln!(writer, "  exit             Exit");
    let _ = writeln!(
        writer,
        "Agent operation is not available here; use `quecto agent` or `quecto-tui`."
    );
}

#[cfg(test)]
mod tests;
