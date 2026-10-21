use super::*;
use crate::shell::signals::TerminationSignal;

fn relaunch(model: Option<&str>, effort: Option<&str>) -> SessionRelaunch {
    SessionRelaunch {
        session_key: "cli:work".into(),
        model: model.map(str::to_string),
        effort: effort.map(str::to_string),
    }
}

fn os(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

#[test]
fn the_relaunch_resumes_the_session_on_its_model_and_effort() {
    let args = relaunch_args(
        os(&[
            "--model",
            "old",
            "--persist",
            "--resume",
            "cli:old",
            "--effort",
            "low",
        ]),
        &relaunch(Some("openai-oauth/gpt-6"), Some("high")),
    );
    assert_eq!(
        args,
        os(&[
            "--persist",
            "--resume",
            "cli:work",
            "--model",
            "openai-oauth/gpt-6",
            "--effort",
            "high"
        ])
    );
    assert_eq!(
        relaunch_args(os(&["--no-workflow"]), &relaunch(None, None)),
        os(&["--no-workflow", "--resume", "cli:work"])
    );
}

/// #2615 review: a termination signal, or no terminal, means no relaunch.
#[test]
fn only_a_requested_restart_onto_a_terminal_without_a_signal_relaunches() {
    assert!(should_relaunch(true, None, true));
    assert!(!should_relaunch(false, None, true));
    assert!(!should_relaunch(
        true,
        Some(TerminationSignal::Hangup),
        true
    ));
    assert!(!should_relaunch(true, None, false));
}

/// #2615 review: after a `cargo install` the running binary's path is gone;
/// `argv[0]` is relaunched instead.
#[test]
fn a_replaced_binary_is_relaunched_through_argv0() {
    let temp = tempfile::tempdir().unwrap();
    let present = temp.path().join("quecto-tui");
    std::fs::write(&present, "").unwrap();
    assert_eq!(
        relaunch_program(Ok(present.clone()), Some("quecto-tui".into())),
        Some(present)
    );
    let deleted = temp.path().join("quecto-tui (deleted)");
    assert_eq!(
        relaunch_program(Ok(deleted), Some("quecto-tui".into())),
        Some(PathBuf::from("quecto-tui"))
    );
    assert_eq!(
        relaunch_program(Err(std::io::ErrorKind::NotFound.into()), None),
        None
    );
}
