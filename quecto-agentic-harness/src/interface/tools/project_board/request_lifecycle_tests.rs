//! #2705 T4: a move's reason and `done_without_run` as both front ends
//! read them. The agent tool's move without a reason is absent, so back to
//! ready from review is refused; the CLI's is unasked, as the Pocket relay
//! sends its moves with none.
use super::*;
use crate::domain::project_board::services::transitions::MoveReason;
use serde_json::json;

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

#[test]
fn a_move_reads_its_reason_as_its_front_end_lets_it() {
    let back = json!({"op": "move", "task": "a", "to": "ready"});
    let reasoned = json!({"op": "move", "task": "a", "to": "ready", "reason": "Plan changed"});
    let cases = [
        (&back, EntryPolicy::Sectioned, MoveReason::Absent),
        (&back, EntryPolicy::SectionedOrLegacy, MoveReason::Unasked),
        (
            &reasoned,
            EntryPolicy::Sectioned,
            MoveReason::Given("Plan changed".into()),
        ),
        (
            &reasoned,
            EntryPolicy::SectionedOrLegacy,
            MoveReason::Given("Plan changed".into()),
        ),
    ];
    for (request, policy, reason) in cases {
        assert_eq!(
            parse(request, policy),
            Ok(BoardRequest::Change(
                slug("a"),
                BoardOp::Move(TaskStatus::Ready, reason)
            )),
            "{request} under {policy:?}"
        );
    }
    let wrong = json!({"op": "move", "task": "a", "to": "ready", "reason": 3});
    let refused = parse(&wrong, EntryPolicy::Sectioned).unwrap_err();
    assert_eq!(
        (refused.field.as_str(), refused.fault.fault),
        ("reason", FaultKind::WrongType)
    );
}

#[test]
fn done_without_run_takes_a_required_reason() {
    let done = json!({"op": "done_without_run", "task": "a", "reason": "Answered in chat"});
    for policy in [EntryPolicy::Sectioned, EntryPolicy::SectionedOrLegacy] {
        assert_eq!(
            parse(&done, policy),
            Ok(BoardRequest::DoneWithoutRun {
                task: slug("a"),
                reason: "Answered in chat".into(),
            })
        );
        let bare = json!({"op": "done_without_run", "task": "a"});
        let refused = parse(&bare, policy).unwrap_err();
        assert_eq!(
            (
                refused.field.as_str(),
                refused.problem.as_str(),
                refused.fault.fault
            ),
            ("reason", "is required", FaultKind::Missing)
        );
    }
}
