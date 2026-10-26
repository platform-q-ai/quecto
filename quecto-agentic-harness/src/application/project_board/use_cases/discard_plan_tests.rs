//! #2481 M4c: a discard discards a draft, keeping it as discarded, and
//! writes it alone.
use super::DiscardPlan;
use crate::application::project_board::dto::ReadPin;
use crate::application::project_board::dto::{PlanDiscard, ProjectBoardError};
use crate::application::project_board::ports::ProjectBoardStore;
use crate::application::project_board::telemetry::OpFacts;
use crate::application::project_board::use_cases::fixtures_tests::{
    SetClock, at, holding_plans, person, plan_of, slug,
};
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::vocabulary::{PlanState, TaskStatus};
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use std::sync::Arc;

fn discard_as(store: &MemoryBoardStore, writer: &str) -> DiscardPlan {
    let client = store.client_as(person(writer));
    DiscardPlan::new(Arc::new(client), SetClock::at("10:00:00")).unwrap()
}

fn of(version: u32) -> PlanDiscard {
    PlanDiscard {
        task: slug("a"),
        version,
        expected: None,
    }
}

fn rule(rule: BoardRuleError) -> Result<(), ProjectBoardError> {
    Err(ProjectBoardError::Rule(rule))
}

#[test]
fn a_discarded_draft_is_kept_and_written_alone() {
    let store = holding_plans(
        "a",
        TaskStatus::Ready,
        vec![plan_of("a", 1, PlanState::Draft)],
    );
    let before = store.read().unwrap();
    let mut facts = OpFacts::default();
    let pinned = PlanDiscard {
        expected: Some(ReadPin::Updated(at("09:00:00"))),
        ..of(1)
    };
    let discarded = discard_as(&store, "Ada")
        .execute_noted(&pinned, Some(&mut facts))
        .unwrap();
    let plan = discarded.fields();
    assert_eq!(
        (plan.state, &plan.updated),
        (PlanState::Discarded, &at("10:00:00"))
    );
    assert_eq!(plan.items, plan_of("a", 1, PlanState::Draft).fields().items);
    let after = store.read().unwrap();
    assert_eq!(after.tasks, before.tasks, "the task is untouched");
    assert_eq!(after.plans, vec![discarded]);
    assert_eq!(
        store.message_at(&after.head).as_deref(),
        Some("board: discard plan 1 of a")
    );
    let want = OpFacts {
        attempts: Some(1),
        lost: Some(0),
        plan_version: Some(1),
        plan_state_from: Some(PlanState::Draft),
        plan_state_to: Some(PlanState::Discarded),
        // The commit the write made (#2663).
        head: Some(crate::application::project_board::ports::BoardHead(
            "r3".into(),
        )),
        ..OpFacts::default()
    };
    assert_eq!(facts, want);
}

/// Only a draft is discarded: never the accepted plan, nor one discarded
/// or superseded already, nor a version the task does not have.
#[test]
fn only_a_known_draft_is_discarded() {
    let store = holding_plans(
        "a",
        TaskStatus::Ready,
        vec![
            plan_of("a", 1, PlanState::Superseded),
            plan_of("a", 2, PlanState::Accepted),
            plan_of("a", 3, PlanState::Discarded),
        ],
    );
    let writes = store.writes();
    let discarding = discard_as(&store, "Ada");
    let refused = |version| discarding.execute(&of(version)).map(|_| ());
    assert_eq!(
        refused(1),
        rule(BoardRuleError::PlanNotDraft(PlanState::Superseded))
    );
    assert_eq!(
        refused(2),
        rule(BoardRuleError::PlanNotDraft(PlanState::Accepted))
    );
    assert_eq!(
        refused(3),
        rule(BoardRuleError::PlanNotDraft(PlanState::Discarded))
    );
    assert_eq!(refused(4), rule(BoardRuleError::UnknownPlanVersion(4)));
    assert_eq!(store.writes(), writes);
    let mut facts = OpFacts::default();
    let _ = discarding.execute_noted(&of(2), Some(&mut facts));
    assert_eq!(
        (facts.plan_state_from, facts.plan_state_to),
        (Some(PlanState::Accepted), None)
    );
}

#[test]
fn a_stale_or_unheld_discard_is_refused() {
    let store = holding_plans(
        "a",
        TaskStatus::Ready,
        vec![plan_of("a", 1, PlanState::Draft)],
    );
    let stale = PlanDiscard {
        expected: Some(ReadPin::Updated(at("08:00:00"))),
        ..of(1)
    };
    assert_eq!(
        discard_as(&store, "Ada").execute(&stale).map(|_| ()),
        rule(BoardRuleError::PlanChanged {
            version: 1,
            updated: at("09:00:00"),
        })
    );
    let done = holding_plans(
        "a",
        TaskStatus::Done,
        vec![plan_of("a", 1, PlanState::Draft)],
    );
    assert_eq!(
        discard_as(&done, "Ada").execute(&of(1)).map(|_| ()),
        rule(BoardRuleError::TaskFinished(TaskStatus::Done))
    );
}

/// Review round 1 (L1): a retry refused before its read notes no state
/// from the attempt that lost.
#[test]
fn a_retry_refused_before_its_read_notes_no_earlier_state() {
    let store = holding_plans(
        "a",
        TaskStatus::Ready,
        vec![plan_of("a", 1, PlanState::Draft)],
    );
    store.after_next_read(|store| store.quarantine("a", "unreadable"));
    let mut facts = OpFacts::default();
    let refused = discard_as(&store, "Ada").execute_noted(&of(1), Some(&mut facts));
    assert!(
        matches!(refused, Err(ProjectBoardError::Quarantined { .. })),
        "{refused:?}"
    );
    assert_eq!((facts.attempts, facts.lost), (Some(2), Some(1)));
    assert_eq!((facts.plan_state_from, facts.plan_state_to), (None, None));
}
