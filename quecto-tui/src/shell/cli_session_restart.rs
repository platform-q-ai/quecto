//! Restarting the session (#2615): when the admission broker serves a
//! different policy, `/admission restart` ends the owned agent as an
//! ordinary exit does (persisting first) and then starts this TUI again,
//! resuming the same session on the same model and effort, so a fresh agent
//! composes against the broker's policy and the conversation carries on.

use std::ffi::OsString;
use std::path::PathBuf;

use crate::shell::app::app_ordinary_exit::SessionRelaunch;

/// The flags a relaunch replaces with the session's own (each takes one
/// value).
const REPLACED: [&str; 3] = ["--resume", "--model", "--effort"];

/// The arguments of the relaunched TUI: the original ones without any
/// `--resume`, `--model` or `--effort`, then the session to resume and the
/// model and effort it was using.
pub(super) fn relaunch_args(
    original: impl IntoIterator<Item = OsString>,
    relaunch: &SessionRelaunch,
) -> Vec<OsString> {
    let mut args = Vec::new();
    let mut original = original.into_iter();
    while let Some(arg) = original.next() {
        match arg.to_str().is_some_and(|arg| REPLACED.contains(&arg)) {
            true => {
                let _value = original.next();
            }
            false => args.push(arg),
        }
    }
    args.extend(["--resume".into(), relaunch.session_key.clone().into()]);
    if let Some(model) = &relaunch.model {
        args.extend(["--model".into(), model.into()]);
    }
    if let Some(effort) = &relaunch.effort {
        args.extend(["--effort".into(), effort.into()]);
    }
    args
}

/// Whether to relaunch at all: only a requested restart, never one a
/// termination signal overtook (the terminal may be gone), and only onto a
/// terminal.
pub(super) fn should_relaunch(
    requested: bool,
    signal: Option<crate::shell::signals::TerminationSignal>,
    stdin_is_terminal: bool,
) -> bool {
    requested && signal.is_none() && stdin_is_terminal
}

/// The program to relaunch: the running binary while it still exists (a
/// `cargo install` over it leaves a `… (deleted)` path), else `argv[0]`,
/// which the exec resolves on `PATH` when it has no slash.
pub(super) fn relaunch_program(
    current_exe: std::io::Result<PathBuf>,
    argv0: Option<OsString>,
) -> Option<PathBuf> {
    match current_exe {
        Ok(path) if path.exists() => Some(path),
        _ => argv0.map(PathBuf::from),
    }
}

/// Replace this process with a fresh TUI resuming `relaunch`. Returns only
/// when that fails, with the exit code to leave with; the reason is on
/// stderr.
pub(super) fn restart_session(relaunch: &SessionRelaunch) -> i32 {
    use std::os::unix::process::CommandExt;
    let manual = format!(
        "start it again with `quecto-tui --resume {}`",
        relaunch.session_key
    );
    let Some(program) = relaunch_program(std::env::current_exe(), std::env::args_os().next())
    else {
        eprintln!("could not restart quecto-tui: its program is unknown; {manual}");
        return 1;
    };
    eprintln!("restarting quecto-tui to use the admission broker's policy…");
    let error = std::process::Command::new(program)
        .args(relaunch_args(std::env::args_os().skip(1), relaunch))
        .exec();
    eprintln!("could not restart quecto-tui: {error}; {manual}");
    1
}

#[cfg(test)]
#[path = "cli_session_restart_tests.rs"]
mod tests;
