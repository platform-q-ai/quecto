//! Epic #2705 B8 review round 1: what a client sends beside a request is
//! read only with the request it belongs to.
use serde_json::json;

use super::*;

fn request(value: Value) -> BoardRequest {
    parse(&value, entry_policy(FrontEnd::Cli)).unwrap()
}

#[test]
fn facts_go_with_an_end_and_an_undo_with_a_start_only() {
    let end = request(json!({"op": "end_run", "task": "a", "run": 1, "outcome": "failed"}));
    let start = request(json!({"op": "start", "task": "a", "plan_version": 1}));
    let facts = json!({"liveness": "unseen", "reports": null});
    let undo = Undo {
        run: 1,
        from: "ready".into(),
        reason: "no runtime".into(),
    };
    let read = served_run(Some(&end), Some(&facts), None);
    assert!(matches!(read, Some(ServedRun::Facts(_))), "{read:?}");
    let none = served_run(Some(&end), None, None);
    assert_eq!(none, Some(ServedRun::Facts(RunFacts::unseen())));
    let undone = served_run(Some(&start), None, Some(&undo));
    assert!(matches!(undone, Some(ServedRun::Undo(_))), "{undone:?}");
    assert_eq!(served_run(Some(&start), None, None), Some(ServedRun::Plain));
    for (request, facts, undo) in [
        (Some(&start), Some(&facts), None),
        (Some(&end), None, Some(&undo)),
        (None, Some(&facts), None),
        (None, None, Some(&undo)),
    ] {
        assert_eq!(served_run(request, facts, undo), None);
    }
    let malformed = json!({"liveness": "maybe", "reports": null});
    assert_eq!(served_run(Some(&end), Some(&malformed), None), None);
    let nowhere = Undo {
        from: "nowhere".into(),
        ..undo
    };
    assert_eq!(served_run(Some(&start), None, Some(&nowhere)), None);
}
