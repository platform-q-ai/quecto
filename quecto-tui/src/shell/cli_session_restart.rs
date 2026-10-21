//! Restarting the session (#2615): when the admission broker came back with
//! a different policy, `/admission restart` ends the owned agent as an
//! ordinary exit does and then starts this TUI again, with the same
//! arguments, so a fresh agent composes against the broker's policy.

use std::ffi::OsString;
use std::path::PathBuf;

/// The command that starts this TUI again with the arguments it ran with.
pub(super) fn session_restart_command(
    program: PathBuf,
    args: impl IntoIterator<Item = OsString>,
) -> std::process::Command {
    let mut command = std::process::Command::new(program);
    command.args(args);
    command
}

/// Replace this process with a fresh TUI. Returns only when that fails,
/// with the exit code to leave with; the reason is on stderr.
pub(super) fn restart_session() -> i32 {
    use std::os::unix::process::CommandExt;
    let program = match std::env::current_exe() {
        Ok(program) => program,
        Err(error) => {
            eprintln!("could not restart quecto-tui: {error}; start it again yourself");
            return 1;
        }
    };
    eprintln!("restarting quecto-tui to use the admission broker's policy…");
    let error = session_restart_command(program, std::env::args_os().skip(1)).exec();
    eprintln!("could not restart quecto-tui: {error}; start it again yourself");
    1
}

#[cfg(test)]
#[path = "cli_session_restart_tests.rs"]
mod tests;
