//! Restarting the session (#2615): when the admission broker serves a
//! different policy, `/admission restart` ends the owned agent as an
//! ordinary exit does (persisting first) and then starts this TUI again with
//! the flags it ran with, resuming the same session on the same model and
//! effort, so a fresh agent composes against the broker's policy and the
//! conversation carries on.

use std::ffi::OsString;
use std::path::PathBuf;

use super::CliFlags;
use crate::shell::app::app_ordinary_exit::SessionRelaunch;

/// The arguments that make `parse_flags` give back `flags`.
pub(super) fn tui_args(flags: &CliFlags) -> Vec<String> {
    let mut args = Vec::new();
    let mut pair = |flag: &str, value: &str| {
        args.push(flag.to_string());
        args.push(value.to_string());
    };
    if let Some(socket) = &flags.socket_path {
        pair("--socket", &socket.to_string_lossy());
    }
    if let Some(config) = &flags.config_path {
        pair("--config", &config.to_string_lossy());
    }
    if let Some(system) = &flags.system_prompt {
        pair("--system", system);
    }
    for tool in &flags.disable_tools {
        pair("--disable-tool", tool);
    }
    if let Some(model) = &flags.model {
        pair("--model", model);
    }
    if let Some(effort) = &flags.effort {
        pair("--effort", effort);
    }
    if let Some(resume) = &flags.resume {
        pair("--resume", resume);
    }
    args.push(
        match flags.persist {
            true => "--persist",
            false => "--no-persist",
        }
        .into(),
    );
    args.push(
        match flags.kill_on_exit {
            true => "--kill-on-exit",
            false => "--detach-on-exit",
        }
        .into(),
    );
    if flags.workflow_disabled {
        args.push("--no-workflow".into());
    }
    if flags.workflow {
        args.push("--workflow".into());
    }
    if flags.workflow_guards {
        args.push("--workflow-guards".into());
    }
    args
}

/// The flags of the relaunched TUI: the ones it ran with, resuming the
/// session on the model and effort it was using.
pub(super) fn relaunch_flags(flags: &CliFlags, relaunch: &SessionRelaunch) -> CliFlags {
    CliFlags {
        resume: Some(relaunch.session_key.clone()),
        model: relaunch.model.clone().or_else(|| flags.model.clone()),
        effort: relaunch.effort.clone().or_else(|| flags.effort.clone()),
        ..flags.clone()
    }
}

/// Whether to relaunch: never when a termination signal overtook the
/// restart (the terminal may be gone), and only onto a terminal.
pub(super) fn should_relaunch(
    signal: Option<crate::shell::signals::TerminationSignal>,
    stdin_is_terminal: bool,
) -> bool {
    signal.is_none() && stdin_is_terminal
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
pub(super) fn restart_session(flags: &CliFlags, relaunch: &SessionRelaunch) -> i32 {
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
        .args(tui_args(&relaunch_flags(flags, relaunch)))
        .exec();
    eprintln!("could not restart quecto-tui: {error}; {manual}");
    1
}

/// The startup the flags ask of the app: whether ordinary exit ends its
/// agent, whether the session is persisted, and the session to resume
/// (#2615).
pub(crate) fn apply_startup_flags(app: &mut crate::shell::app::App, flags: &CliFlags) {
    app.set_ordinary_exit_kill_owned(flags.kill_on_exit);
    app.set_session_persisted(flags.persist);
    if let Some(session) = &flags.resume {
        app.send_resume_session(session);
    }
}

/// After the app's loop: relaunch the TUI when it asked to restart the
/// session and nothing forbids it; the exit code when it did not relaunch
/// for want of a terminal or because a signal overtook it is the caller's.
pub(super) fn after_exit(app: &crate::shell::app::App, flags: &CliFlags) -> Option<i32> {
    use std::io::IsTerminal;
    let relaunch = app.session_relaunch()?;
    if should_relaunch(app.termination_signal(), std::io::stdin().is_terminal()) {
        return Some(restart_session(flags, relaunch));
    }
    eprintln!(
        "not restarting quecto-tui (no terminal, or it was asked to stop); start it again with `quecto-tui --resume {}`",
        relaunch.session_key
    );
    None
}

#[cfg(test)]
#[path = "cli_session_restart_tests.rs"]
mod tests;
