//! Epic #2705 B8: a begun start is read from its server's answer for the
//! fields named alone, and the launched answer is the server's with the
//! coordinator in place of what it was launched with.
use serde_json::json;

use super::*;

fn start() -> TaskStart {
    TaskStart {
        task: Slug::parse("a").unwrap(),
        plan_version: 1,
    }
}

/// A server's begun answer, as it renders one.
fn begun_answer() -> Value {
    json!({
        "ok": true, "op": "start",
        "task": {"id": "a", "status": "in_progress"},
        "run": {"task": "a", "number": 2, "state": "running"},
        "from": "review",
        "launch": {"plan_version": 1, "brief": "Coordinate a.", "model": "gpt-5.5", "effort": "high"},
        "not_applied": [{"role": "builder", "model": "x", "effort": "low"}],
        "claim": "held",
        "renew_after": "2026-10-25T11:00:00Z",
        "steps": "Once it has started…"
    })
}

#[test]
fn a_begun_start_is_read_for_what_its_coordinator_is_launched_with() {
    let begun = begun_of(&start(), begun_answer()).expect("a begun start");
    assert_eq!(
        begun.launch,
        CoordinatorRequest {
            task: Slug::parse("a").unwrap(),
            run: 2,
            plan_version: 1,
            brief: "Coordinate a.".into(),
            model: Some("gpt-5.5".into()),
            effort: Some(Effort::High),
        }
    );
    assert_eq!(
        (begun.from, begun.claim),
        (TaskStatus::Review, StartClaim::Held)
    );
    assert_eq!(
        (begun.noted.run, begun.noted.not_applied, begun.noted.claim),
        (Some(2), Some(1), Some(StartClaim::Held))
    );
    let mut session = begun_answer();
    session["launch"]["model"] = Value::Null;
    session["launch"]["effort"] = Value::Null;
    let begun = begun_of(&start(), session).expect("a begun start");
    assert_eq!((begun.launch.model, begun.launch.effort), (None, None));
}

#[test]
fn an_answer_that_is_no_begun_start_is_none() {
    let breaks: [fn(&mut Value); 6] = [
        |answer| answer["task"]["id"] = json!("b"),
        |answer| answer["run"]["number"] = json!(-1),
        |answer| answer["from"] = json!("nowhere"),
        |answer| answer["claim"] = json!("taken"),
        |answer| answer["launch"]["effort"] = json!("extreme"),
        |answer| answer["launch"]["brief"] = Value::Null,
    ];
    for (at, broken) in breaks.iter().enumerate() {
        let mut answer = begun_answer();
        broken(&mut answer);
        assert!(begun_of(&start(), answer).is_none(), "break {at}");
    }
}

#[test]
fn a_launched_answer_names_its_coordinator_in_place_of_its_launch() {
    let coordinator = LaunchedCoordinator {
        agent: "c-1".into(),
        environment_ref: "env-1".into(),
    };
    let answer = launched_answer(begun_answer(), &coordinator);
    let answer: Value = serde_json::from_str(&answer.0).unwrap();
    let keys: Vec<&str> = answer
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "ok",
            "op",
            "task",
            "run",
            "coordinator",
            "not_applied",
            "claim",
            "renew_after",
            "steps"
        ],
        "a local start's shape"
    );
    assert_eq!(
        answer["coordinator"],
        json!({"agent": "c-1", "environment_ref": "env-1"})
    );
}

/// A launcher whose launch is refused.
struct Refusing;

impl crate::application::project_board::ports::CoordinatorLaunch for Refusing {
    fn may_launch(&self) -> bool {
        true
    }

    fn launch(
        &self,
        _: &CoordinatorRequest,
        _: &std::sync::atomic::AtomicBool,
    ) -> Result<LaunchedCoordinator, crate::application::project_board::ports::LaunchRefused> {
        Err(crate::application::project_board::ports::LaunchRefused::because("no runtime"))
    }
}

const FOLDED: &str = r#"{"answer":{"ok":false,"op":"start","error":{"code":"launch_failed",
    "message":"m","run":2,"run_state":"folded"}},"facts":null}"#;
const NOT_HOLDER: &str = r#"{"answer":{"ok":false,"op":"start","error":{"code":"rule",
    "message":"only the claim's holder may do this","rule":{"code":"not_holder"}}},"facts":null}"#;
const NOT_LATEST: &str = r#"{"answer":{"ok":false,"op":"start","error":{"code":"rule",
    "message":"the task's runs contradict: run 2 is not the task's latest run",
    "rule":{"code":"runs_inconsistent"}}},"facts":null}"#;

/// Review round 2: an undo whose answer never came is sent once more, the
/// same (L4); an undo the server refused says the server's code and words
/// in the start's `launch_failed` (M2), never that the run waits for
/// `end_run` while the task is yours.
#[test]
fn an_undo_is_resent_once_and_a_refused_one_says_why() {
    use super::super::board_remote::tests::holding;
    use crate::application::project_board::use_cases::LaunchBegunRun;
    let ada = crate::domain::project_board::entities::claim::Identity {
        name: "Ada".into(),
        email: "ada@example.com".into(),
    };
    let board =
        crate::domain::project_board::value_objects::telemetry::BoardId::of_hex("0123456789abcdef")
            .unwrap();
    for (held, answer, state, refused) in [
        (1, FOLDED, RunAfterUndo::Folded, None),
        (
            0,
            NOT_HOLDER,
            RunAfterUndo::Refused,
            Some("rule (not_holder)"),
        ),
        (
            0,
            NOT_LATEST,
            RunAfterUndo::Refused,
            Some("rule (runs_inconsistent)"),
        ),
    ] {
        let (_dir, client, bodies) = holding(held, (200, answer));
        let mut remote = RemoteBoardOperations::new(vec![client], board.clone(), Some(ada.clone()));
        remote.replies.1 = std::time::Duration::from_millis(300);
        let runs = Arc::new(RemoteBoardRuns(Arc::new(remote)));
        let launch = LaunchBegunRun::new(runs, Some(Arc::new(Refusing)), None);
        let begun = begun_of(&start(), begun_answer()).unwrap();
        let error = launch.launch(begun, None).unwrap_err();
        let ProjectBoardError::LaunchFailed {
            reason, run_state, ..
        } = &error
        else {
            panic!("{error:?}")
        };
        assert_eq!(*run_state, state, "{error}");
        let said = error.to_string();
        match refused {
            Some(code) => {
                assert!(reason.contains(code), "{reason}");
                assert!(!said.contains("you still hold the task"), "{said}");
            }
            None => {
                let bodies = bodies.lock().unwrap();
                assert_eq!(
                    (bodies.len(), &bodies[0]),
                    (2, &bodies[1]),
                    "resent the same"
                );
            }
        }
    }
}
