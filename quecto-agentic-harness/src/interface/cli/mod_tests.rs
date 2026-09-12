use super::*;

fn args(command: &str) -> Vec<String> {
    std::iter::once("quecto".to_owned())
        .chain(command.split_whitespace().map(str::to_owned))
        .collect()
}

#[test]
fn live_cli_commands_keep_help_version_and_unknown_dispatch() {
    let help = run_with_output(args("help"), &CliContext::default());
    assert_eq!(help.exit_code, 0);
    assert!(help.stdout.contains("Usage: quecto [command]"));
    assert!(help.stdout.contains("agent"));

    let version = run_with_output(args("--version"), &CliContext::default());
    assert_eq!(version.exit_code, 0);
    assert!(version.stdout.contains(env!("CARGO_PKG_VERSION")));

    let unknown = run_with_output(args("not-a-command"), &CliContext::default());
    assert_eq!(unknown.exit_code, 1);
    assert!(unknown.stderr.contains("Unknown command: not-a-command"));
}

fn test_composition() -> CliComposition {
    CliComposition {
        web_fetch_tool_factory: crate::composition::web_fetch::build,
        teardown_graph: crate::composition::subagent_teardown::build_teardown_graph,
    }
}

#[test]
fn real_run_dispatches_non_repl_commands() {
    let composition = test_composition();
    assert_eq!(run(args("version"), composition), 0);
    assert_eq!(run(args("definitely-not-a-command"), composition), 1);
}

#[test]
fn public_run_accepts_required_web_fetch_factory_when_fetch_is_enabled() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config.json");
    std::fs::write(
        &config,
        r#"{
            "providers":{"openai":{"api_key":"sk-test","api_base":"http://127.0.0.1:1"}},
            "tools":{"web":{"fetch":{"enabled":true}}}
        }"#,
    )
    .unwrap();
    let result = std::panic::catch_unwind(|| {
        run(
            vec![
                "quecto".into(),
                "--config".into(),
                config.display().to_string(),
                "agent".into(),
                "--no-session".into(),
                "--message".into(),
                "hello".into(),
                "--max-iterations".into(),
                "1".into(),
            ],
            test_composition(),
        )
    });
    assert!(
        result.is_ok(),
        "public run must not panic with fetch enabled"
    );
}

/// The blocking pool keeps a resident thread: two sequential blocking tasks
/// run on the same OS thread instead of each creating (and retiring) one.
#[test]
fn harness_runtime_keeps_a_resident_blocking_thread() {
    let runtime = super::build_tokio_runtime().unwrap();
    let blocking_thread = || async {
        tokio::task::spawn_blocking(|| std::thread::current().id())
            .await
            .unwrap()
    };
    let first = runtime.block_on(blocking_thread());
    std::thread::sleep(std::time::Duration::from_millis(20));
    let second = runtime.block_on(blocking_thread());
    assert_eq!(first, second, "the warm thread is reused, never retired");
}

/// The test-support execution-state and ledger-hint probes are part of the
/// lib's public surface for the BDD suites; pin their shapes here too.
#[tokio::test]
async fn test_support_probes_report_live_and_completed_execution_state() {
    use crate::domain::agent::AgentProgressEvent;
    let started = AgentProgressEvent::ToolStarted {
        tool_call_id: "c1".into(),
        name: "bash".into(),
        arguments: "{}".into(),
    };
    let live = super::live_execution_state_for_events(std::slice::from_ref(&started));
    assert_eq!(live["execution"]["phase"], "runningTool");
    assert_eq!(live["execution"]["tools"]["started"], 1);
    let done = super::completed_live_execution_state(&[started]);
    assert_eq!(done["execution"]["phase"], "idle");
    assert_eq!(done["messageCount"], 0);
    let hints = super::ledger_hint_lines_for_turn_events(&[AgentProgressEvent::Done]).await;
    assert!(
        hints.iter().all(|line| line.is_object()),
        "every emitted line is a JSON event: {hints:?}"
    );
}
