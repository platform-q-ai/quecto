//! Operator H1 contracts through the production registry and CLI session runner.
use super::*;

/// A hard-killed child cannot leave scratch files behind: its parent owns TMPDIR.
/// The child inherits core=0 even when this test executable is invoked directly.
fn isolated(test: &str, switches: &[&str], roots: &[&str]) -> bool {
    if std::env::var("QUECTO_RUN16_CLI_CASE").ok().as_deref() == Some(test) {
        return true;
    }
    let scratch = tempfile::TempDir::new().expect("parent-owned scratch");
    let (_, selection) = test.split_once("::").expect("crate-qualified test");
    let mut failures = Vec::new();
    for switch in switches {
        for root in roots {
            let mut command = std::process::Command::new("sh");
            command.args([
                "-c",
                "ulimit -c 0; exec timeout --signal=KILL 45s \"$@\"",
                "run16-cli",
            ]);
            command.arg(std::env::current_exe().expect("test executable"));
            command.args(["--exact", selection, "--nocapture"]);
            command.env("QUECTO_RUN16_CLI_CASE", test);
            command.env("QUECTO_2708_TEST_ROOT", root);
            command.env("TMPDIR", scratch.path());
            command.env_remove("QUECTO_TURN_OUTCOMES");
            if matches!(*switch, "1" | "other") {
                command.env("QUECTO_TURN_OUTCOMES", switch);
            }
            let output = command.output().expect("bounded child");
            let stdout = String::from_utf8_lossy(&output.stdout);
            println!(
                "H1 root={root} switch={switch}: {}\n{stdout}\n{}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                stdout.contains("running 1 test\n"),
                "nonzero exact selection: {selection}"
            );
            match output.status.code() {
                Some(0) => {}
                _ => failures.push(format!("{root}/{switch}: {}", output.status)),
            }
        }
    }
    // Explicit close also verifies cleanup after assertion failures in a child.
    scratch.close().expect("scratch cleanup");
    assert!(failures.is_empty(), "H1 child failures: {failures:?}");
    false
}

pub(super) fn bounded(test: &str) -> bool {
    isolated(test, &["unset"], &["default"])
}

fn matrix(test: &str) -> bool {
    isolated(
        test,
        &["unset", "1", "other"],
        &["default", "named", "ephemeral"],
    )
}

fn production(replies: Vec<Reply>) -> Run {
    use crate::interface::cli::controllers::agent::active_runtime_2708_tests::compose;
    run_composition(
        replies,
        (false, vec![], None),
        (None, true),
        Some(compose(false)),
    )
}

fn declaration(text: &'static str, outcome: &str, reason: &str) -> Reply {
    Reply::TextAndCall(
        text,
        "declare_turn_outcome",
        json!({"outcome":outcome,"reason":reason}),
    )
}

fn assert_requests(run: &Run, expected: usize, corrections: usize) {
    assert_eq!(
        run.requests.len(),
        expected,
        "finite provider script: {:?}",
        run.requests
    );
    assert_eq!(run.correction_messages.len(), expected);
    assert_eq!(
        run.correction_messages.last(),
        Some(&corrections),
        "actual correction messages in final provider request"
    );
    assert!(
        run.correction_messages
            .iter()
            .all(|count| *count <= corrections),
        "no request ever contains an extra correction: {:?}",
        run.correction_messages
    );
}

/// Preserve every assertion while executing all independent fixture scenarios.
fn aggregate<T: Copy + std::fmt::Debug>(cases: &[T], mut check: impl FnMut(T)) {
    let mut failures = Vec::new();
    for &case in cases {
        println!("H1 scenario={case:?}");
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| check(case))) {
            Ok(()) => {}
            Err(_) => failures.push(format!("{case:?}")),
        }
    }
    assert!(
        failures.is_empty(),
        "independent H1 scenario failures: {failures:?}"
    );
}

#[test]
fn cold_review_h1_text_and_declaration_then_empty_retains_answer() {
    if matrix(concat!(
        module_path!(),
        "::cold_review_h1_text_and_declaration_then_empty_retains_answer"
    )) {
        // Indispensable fixture: text AND declaration in ONE reply, then EMPTY.
        let run = production(vec![
            declaration("the answer", "done", "Answered"),
            Reply::Text(""),
        ]);
        assert_requests(&run, 2, 0);
        assert_eq!(run.stdout, "the answer\n");
        assert_eq!(run.code, 0);
        assert_eq!(run.stderr, "turn outcome: done: Answered\n");
    }
}

#[test]
fn cold_review_h1_last_nonempty_text_only() {
    if matrix(concat!(
        module_path!(),
        "::cold_review_h1_last_nonempty_text_only"
    )) {
        let run = production(vec![
            declaration("older answer", "done", "First"),
            declaration("  newest answer  ", "done", "Newest"),
            Reply::Text(" \t"),
        ]);
        assert_requests(&run, 3, 0);
        assert_eq!(
            run.stdout, "  newest answer  \n",
            "preserve bytes, never concatenate or select whitespace/tool results"
        );
        assert_eq!(run.code, 0);
        assert_eq!(run.stderr, "turn outcome: done: Newest\n");
    }
}

#[test]
fn cold_review_h1_completed_outcomes_exit_zero() {
    if matrix(concat!(
        module_path!(),
        "::cold_review_h1_completed_outcomes_exit_zero"
    )) {
        aggregate(
            &[
                ("done", "done"),
                ("asking", "needs_user"),
                ("blocked", "needs_user"),
            ],
            |(outcome, settled)| {
                let run = production(vec![
                    declaration("answer", outcome, "A reason"),
                    Reply::Text("answer"),
                ]);
                assert_requests(&run, 2, 0);
                assert_eq!(run.stdout, "answer\n");
                assert_eq!(run.code, 0, "completed {outcome} is not a master error");
                assert_eq!(run.stderr, format!("turn outcome: {settled}: A reason\n"));
            },
        );
    }
}

#[test]
fn cold_review_h1_empty_correction_does_not_erase_episode_text() {
    if matrix(concat!(
        module_path!(),
        "::cold_review_h1_empty_correction_does_not_erase_episode_text"
    )) {
        let run = production(vec![
            Reply::Text("current answer"),
            declaration("", "done", "Corrected"),
            Reply::Text(""),
        ]);
        assert_requests(&run, 3, 1);
        assert_eq!(run.stdout, "current answer\n");
        assert_eq!(run.code, 0);
        assert_eq!(run.stderr, "turn outcome: done: Corrected\n");
    }
}

#[test]
fn cold_review_h1_stalled_is_documented_exception() {
    if matrix(concat!(
        module_path!(),
        "::cold_review_h1_stalled_is_documented_exception"
    )) {
        aggregate(&[PROMISE_2648, PROMISE_SPLIT_READER], |promise| {
            assert_stalled(
                production(vec![Reply::Text(promise), Reply::Text(promise)]),
                promise,
            );
        });
    }
}

#[test]
fn cold_review_h1_legacy_empty_after_tool_retains_text() {
    if bounded(concat!(
        module_path!(),
        "::cold_review_h1_legacy_empty_after_tool_retains_text"
    )) {
        // Ordinary launch tool, no declaration authority or guard diagnostics.
        let run = run(
            vec![
                Reply::TextAndCall("legacy answer", "launch", json!({})),
                Reply::Text(""),
            ],
            true,
            vec![],
            None,
        );
        assert_requests(&run, 2, 0);
        assert_eq!(run.stdout, "legacy answer\n");
        assert_eq!((run.code, run.stderr.as_str()), (0, ""));
    }
}

#[test]
fn cold_review_h1_provider_failure_and_timeout_keep_error_codes() {
    if matrix(concat!(
        module_path!(),
        "::cold_review_h1_provider_failure_and_timeout_keep_error_codes"
    )) {
        let failed = production(vec![Reply::Failure]);
        assert_requests(&failed, 1, 0);
        assert_eq!(failed.code, 1);
        assert_eq!(failed.stdout, "");
        assert!(failed.stderr.contains("scripted failure"));
        let timed = run_composition(
            vec![Reply::Call("launch", json!({})), Reply::Text("Launched.")],
            (false, vec![], Some("1")),
            (None, false),
            Some(
                crate::interface::cli::controllers::agent::active_runtime_2708_tests::compose(
                    false,
                ),
            ),
        );
        assert_requests(&timed, 2, 0);
        assert_eq!(timed.code, 2);
        assert_eq!(timed.stdout, "");
        assert!(timed.stderr.contains("max-time exceeded"));
    }
}

#[test]
fn cold_review_h1_no_old_session_text_when_turn_is_empty() {
    if isolated(
        concat!(
            module_path!(),
            "::cold_review_h1_no_old_session_text_when_turn_is_empty"
        ),
        &["unset", "1", "other"],
        &["default", "named"],
    ) {
        let run = run_composition_history(
            vec![
                declaration("", "done", "Empty current turn"),
                Reply::Text(""),
            ],
            (false, vec![], None),
            (None, true),
            Some(
                crate::interface::cli::controllers::agent::active_runtime_2708_tests::compose(
                    false,
                ),
            ),
            Some("OLD SESSION ANSWER MUST NOT PRINT"),
        );
        assert_requests(&run, 2, 0);
        assert!(
            run.assistant_history
                .iter()
                .any(|text| text == "OLD SESSION ANSWER MUST NOT PRINT"),
            "real saved history reached provider"
        );
        assert_eq!(
            run.stdout, "\n",
            "no old session answer for an empty current invocation"
        );
        assert_eq!(run.code, 0);
        assert_eq!(run.stderr, "turn outcome: done: Empty current turn\n");
    }
}

/// A one-shot driver consumes Waiting by waiting for notes, so pin its CLI
/// presenter separately using the production runtime and observed owned work.
#[test]
fn cold_review_h1_completed_waiting_presenter_exits_zero() {
    if matrix(concat!(
        module_path!(),
        "::cold_review_h1_completed_waiting_presenter_exits_zero"
    )) {
        use crate::application::tools::ports::Tool;
        let composed =
            crate::interface::cli::controllers::agent::active_runtime_2708_tests::compose(false);
        let runtime = composed.runtime.expect("production eligible root");
        let registry = new_registry();
        let rt = crate::infrastructure::processes::tokio_runtime::build_tokio_runtime().unwrap();
        rt.block_on(LaunchTool(registry.clone()).execute("{}"))
            .unwrap();
        let parent = ParentRun::of(Some(runtime.clone()), Some(registry), None).unwrap();
        let turn = runtime
            .admit(
                "waiting-presenter",
                TurnAdmission::External {
                    episode_id: "waiting-episode".into(),
                },
                "waiting-turn",
            )
            .unwrap();
        let ParentVerdict::Published(record) =
            runtime.complete(turn, TurnEnd::Completed(parent.observe()))
        else {
            panic!("actual owned running work publishes Waiting");
        };
        assert_eq!(record.outcome, TurnOutcome::Waiting);
        let (mut stdout, mut stderr) = (String::new(), String::new());
        let code = exit_code(
            Some(&record),
            &mut AgentOutput {
                stdout: &mut stdout,
                stderr: &mut stderr,
            },
        );
        assert_eq!(
            code, 0,
            "Waiting is completed admission, not a master error"
        );
        assert_eq!(stdout, "");
        assert_eq!(
            stderr,
            format!("turn outcome: waiting: {}\n", record.reason)
        );
    }
}
