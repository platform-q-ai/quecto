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
