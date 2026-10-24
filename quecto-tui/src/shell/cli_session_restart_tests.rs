use super::*;
use crate::shell::signals::TerminationSignal;

fn parse(args: &[&str]) -> CliFlags {
    let mut argv = vec!["quecto-tui".to_string()];
    argv.extend(args.iter().map(|arg| arg.to_string()));
    super::super::parse_flags(&argv)
}

fn relaunch(model: Option<&str>, effort: Option<&str>) -> SessionRelaunch {
    SessionRelaunch {
        session_key: "cli:work".into(),
        model: model.map(str::to_string),
        effort: effort.map(str::to_string),
    }
}

/// #2615 review round 2: the relaunch is rebuilt from the parsed flags, and
/// parses back to exactly them.
#[test]
fn the_relaunch_flags_round_trip_through_the_arguments() {
    for args in [
        &[][..],
        &["--model", "old", "--no-persist", "--detach-on-exit"][..],
        &[
            "--socket",
            "/s.sock",
            "--config",
            "/c.json",
            "--system",
            "be brief",
            "--disable-tool",
            "bash",
            "--disable-tool",
            "grep",
            "--effort",
            "low",
            "--resume",
            "cli:old",
            "--workflow",
            "--workflow-guards",
        ][..],
        &["--no-workflow"][..],
    ] {
        let flags = parse(args);
        let args_back = tui_args(&flags);
        let argv: Vec<&str> = args_back.iter().map(String::as_str).collect();
        assert_eq!(parse(&argv), flags, "{args:?}");
    }
}

#[test]
fn the_relaunch_resumes_the_session_on_its_model_and_effort() {
    let flags = parse(&["--model", "old", "--resume", "cli:old", "--no-workflow"]);
    let next = relaunch_flags(&flags, &relaunch(Some("openai-oauth/gpt-6"), Some("high")));
    assert_eq!(next.resume.as_deref(), Some("cli:work"));
    assert_eq!(next.model.as_deref(), Some("openai-oauth/gpt-6"));
    assert_eq!(next.effort.as_deref(), Some("high"));
    assert!(next.workflow_disabled, "the rest carries over");
    let kept = relaunch_flags(&flags, &relaunch(None, None));
    assert_eq!(
        kept.model.as_deref(),
        Some("old"),
        "an unknown model keeps the flag"
    );
}

/// #2615 review: a termination signal, or no terminal, means no relaunch.
#[test]
fn only_a_restart_onto_a_terminal_without_a_signal_relaunches() {
    assert!(should_relaunch(None, true));
    assert!(!should_relaunch(Some(TerminationSignal::Hangup), true));
    assert!(!should_relaunch(None, false));
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

/// #2615 review round 2: `--resume` is parsed, and a missing value is
/// ignored like `--model`'s.
#[test]
fn resume_is_parsed_and_a_missing_value_is_ignored() {
    assert_eq!(
        parse(&["--resume", "cli:work"]).resume.as_deref(),
        Some("cli:work")
    );
    assert_eq!(parse(&["--resume"]).resume, None);
    let flags = parse(&["--resume", "--no-persist"]);
    assert_eq!(flags.resume, None);
    assert!(!flags.persist, "the next flag is still read");
}

/// #2615 review round 2: a startup with `--resume` resumes that session.
#[tokio::test]
async fn a_startup_with_resume_sends_resume_session() {
    let mut h = crate::shell::app::tui_harness::TuiHarness::new().await;
    super::apply_startup_flags(h.app_mut(), &parse(&["--resume", "cli:work"]));
    let sent = h.drain_commands().await;
    assert!(
        sent.iter()
            .any(|c| c.contains(r#""type":"resume_session""#) && c.contains("cli:work")),
        "{sent:?}"
    );
    let mut h = crate::shell::app::tui_harness::TuiHarness::new().await;
    super::apply_startup_flags(h.app_mut(), &parse(&[]));
    assert!(
        !h.drain_commands()
            .await
            .iter()
            .any(|c| c.contains("resume_session")),
        "nothing resumed without the flag"
    );
}

/// #2615 review round 3: `--resume=<key>` carries a key that starts with
/// `-`, and the relaunch uses that form.
#[test]
fn a_key_starting_with_a_dash_resumes_through_the_one_word_form() {
    assert_eq!(
        parse(&["--resume=--legacy"]).resume.as_deref(),
        Some("--legacy")
    );
    assert_eq!(
        parse(&["--resume=cli:work"]).resume.as_deref(),
        Some("cli:work")
    );
    assert_eq!(
        parse(&["--resume="]).resume,
        None,
        "an empty key resumes nothing"
    );
    let flags = relaunch_flags(
        &parse(&[]),
        &SessionRelaunch {
            session_key: "--legacy".into(),
            model: None,
            effort: None,
        },
    );
    let args = tui_args(&flags);
    assert!(args.contains(&"--resume=--legacy".to_string()), "{args:?}");
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();
    assert_eq!(parse(&argv).resume.as_deref(), Some("--legacy"));
}

/// #2615 review round 3: a system prompt from a file is re-read from that
/// file, never passed inline; one given with `--system` stays inline; a
/// workflow's default prompt is not passed at all.
#[test]
fn the_system_prompt_is_relaunched_as_the_user_gave_it() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("system.md");
    std::fs::write(&file, "a long system prompt").unwrap();
    let from_file = parse(&["--system-file", file.to_str().unwrap()]);
    assert_eq!(
        from_file.system_prompt.as_deref(),
        Some("a long system prompt")
    );
    let args = tui_args(&from_file);
    assert!(args.iter().any(|arg| arg == "--system-file"), "{args:?}");
    assert!(!args.iter().any(|arg| arg == "--system"), "{args:?}");
    assert!(
        !args.iter().any(|arg| arg.contains("long system prompt")),
        "{args:?}"
    );
    let argv: Vec<&str> = args.iter().map(String::as_str).collect();
    assert_eq!(parse(&argv), from_file);
    let literal = parse(&["--system", "be brief"]);
    let args = tui_args(&literal);
    assert!(
        args.windows(2).any(|w| w == ["--system", "be brief"]),
        "{args:?}"
    );
    let mut workflow = parse(&["--workflow"]);
    super::super::apply_workflow_defaults(&mut workflow);
    assert!(
        workflow.system_prompt.is_some(),
        "the workflow's default prompt"
    );
    let args = tui_args(&workflow);
    assert!(
        !args
            .iter()
            .any(|arg| arg == "--system" || arg == "--system-file"),
        "{args:?}"
    );
}
