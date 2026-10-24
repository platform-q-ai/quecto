//! #2481 M4b: an edit of a draft is a patch: what it leaves out stays as
//! the board holds it, byte for byte, and a stale edit is refused.
use super::EditPlan;
use crate::application::project_board::dto::{PlanEdit, ProjectBoardError};
use crate::application::project_board::ports::ProjectBoardStore;
use crate::application::project_board::telemetry::OpFacts;
use crate::application::project_board::use_cases::fixtures_tests::{
    SetClock, at, coordinated_team, holding_plans, person, plan_item, plan_of, slug,
    unqualified_coordinator,
};
use crate::domain::project_board::entities::plan::{Plan, PlanEntry, PlanFields};
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::telemetry::{Normalised, TextChange};
use crate::domain::project_board::value_objects::vocabulary::{PlanState, TaskStatus};
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use std::sync::Arc;

fn edit_as(store: &MemoryBoardStore, writer: &str) -> EditPlan {
    let client = store.client_as(person(writer));
    EditPlan::new(Arc::new(client), SetClock::at("10:00:00")).unwrap()
}

/// An edit of draft 1 of task `a` setting nothing yet.
fn of(version: u32) -> PlanEdit {
    PlanEdit {
        handoff: None,
        entry: PlanEntry::Legacy,
        task: slug("a"),
        version,
        approach: None,
        items: None,
        team: None,
        expected_updated: None,
    }
}

/// Draft 1 of task `a`, its approach near its 64 Ki-character limit (more
/// than a tool's `show` gives beside the task), spaced inside its code.
fn long_draft() -> Plan {
    let words = vec!["word"; 13_000].join(" ");
    let approach = format!("Spike first.\n\n```\n  keep  spaced\n```\n\n{words}");
    Plan::new(PlanFields {
        approach,
        ..plan_of("a", 1, PlanState::Draft).into_fields()
    })
    .unwrap()
}

#[test]
fn an_edit_of_the_items_alone_keeps_the_approach_byte_for_byte() {
    let store = holding_plans("a", TaskStatus::Ready, vec![long_draft()]);
    let edit = PlanEdit {
        handoff: None,
        entry: PlanEntry::Legacy,
        items: Some(vec![plan_item("only", &[])]),
        expected_updated: Some(at("09:00:00")),
        ..of(1)
    };
    let edited = edit_as(&store, "Cy").execute(&edit).unwrap();
    let (held, plan) = (long_draft().into_fields(), edited.fields());
    assert_eq!(plan.approach.as_bytes(), held.approach.as_bytes());
    assert_eq!(plan.team, held.team, "the team is kept too");
    assert_eq!(plan.items, vec![plan_item("only", &[])]);
    assert_eq!(
        (&plan.proposed_by, &plan.edited_by, &plan.updated),
        (&person("Bob"), &Some(person("Cy")), &at("10:00:00"))
    );
    let after = store.read().unwrap();
    assert_eq!(after.plans, vec![edited]);
    assert_eq!(
        store.message_at(&after.head).as_deref(),
        Some("board: edit plan 1 of a")
    );
}

/// A field given replaces the draft's whole: the team taken away by an
/// explicit none, the approach normalised as it enters.
#[test]
fn a_field_given_replaces_the_drafts_whole() {
    let store = holding_plans(
        "a",
        TaskStatus::Ready,
        vec![plan_of("a", 1, PlanState::Draft)],
    );
    let edit = PlanEdit {
        handoff: None,
        entry: PlanEntry::Legacy,
        approach: Some("New.\r\n".into()),
        team: Some(None),
        ..of(1)
    };
    let mut facts = OpFacts::default();
    let edited = edit_as(&store, "Ada")
        .execute_noted(&edit, Some(&mut facts))
        .unwrap();
    let plan = edited.fields();
    assert_eq!((plan.approach.as_str(), &plan.team), ("New.", &None));
    assert_eq!(plan.items, plan_of("a", 1, PlanState::Draft).fields().items);
    let want = OpFacts {
        attempts: Some(1),
        lost: Some(0),
        normalised: Some(Normalised {
            title: None,
            description: None,
            approach: Some(vec![TextChange::Crlf, TextChange::EdgeBlankLines]),
            items: None,
        }),
        plan_version: Some(1),
        items: Some(2),
        roles: Some(0),
        // The commit the write made (#2663).
        head: Some(crate::application::project_board::ports::BoardHead(
            "r3".into(),
        )),
        ..OpFacts::default()
    };
    assert_eq!(facts, want);
}

#[test]
fn a_stale_expected_updated_is_refused_as_plan_changed() {
    let store = holding_plans(
        "a",
        TaskStatus::Ready,
        vec![plan_of("a", 1, PlanState::Draft)],
    );
    let writes = store.writes();
    let edit = PlanEdit {
        handoff: None,
        entry: PlanEntry::Legacy,
        items: Some(vec![plan_item("only", &[])]),
        expected_updated: Some(at("08:00:00")),
        ..of(1)
    };
    let mut facts = OpFacts::default();
    assert_eq!(
        edit_as(&store, "Ada").execute_noted(&edit, Some(&mut facts)),
        Err(ProjectBoardError::Rule(BoardRuleError::PlanChanged {
            version: 1,
            updated: at("09:00:00"),
        }))
    );
    assert_eq!(store.writes(), writes, "nothing written");
    assert_eq!((facts.plan_version, facts.items), (Some(1), None));
}

/// A lost race rebuilds the patch onto the fresh draft: another writer's
/// approach, written in between, is kept, never written back over.
#[test]
fn a_lost_race_patches_the_fresh_draft() {
    let store = holding_plans(
        "a",
        TaskStatus::Ready,
        vec![plan_of("a", 1, PlanState::Draft)],
    );
    let other = store.client();
    store.after_next_read(move |_| {
        let theirs = PlanEdit {
            handoff: None,
            entry: PlanEntry::Legacy,
            approach: Some("Theirs.".into()),
            ..of(1)
        };
        edit_as(&other, "Bob").execute(&theirs).unwrap();
    });
    let mine = PlanEdit {
        handoff: None,
        entry: PlanEntry::Legacy,
        items: Some(vec![plan_item("mine", &[])]),
        ..of(1)
    };
    let mut facts = OpFacts::default();
    let edited = edit_as(&store, "Cy")
        .execute_noted(&mine, Some(&mut facts))
        .unwrap();
    let plan = edited.fields();
    assert_eq!(plan.approach, "Theirs.");
    assert_eq!(plan.items, vec![plan_item("mine", &[])]);
    assert_eq!((facts.attempts, facts.lost), (Some(2), Some(1)));
}

#[test]
fn only_a_known_draft_of_an_open_task_is_edited_and_only_by_a_change() {
    let items = Some(vec![plan_item("only", &[])]);
    let store = holding_plans(
        "a",
        TaskStatus::Ready,
        vec![
            plan_of("a", 1, PlanState::Superseded),
            plan_of("a", 2, PlanState::Draft),
        ],
    );
    let editor = edit_as(&store, "Ada");
    let refused = |edit: PlanEdit| editor.execute(&edit).map(|_| ());
    let rule = |rule| Err(ProjectBoardError::Rule(rule));
    let unknown = PlanEdit {
        handoff: None,
        entry: PlanEntry::Legacy,
        items: items.clone(),
        ..of(9)
    };
    assert_eq!(
        refused(unknown),
        rule(BoardRuleError::UnknownPlanVersion(9))
    );
    let settled = PlanEdit {
        handoff: None,
        entry: PlanEntry::Legacy,
        items: items.clone(),
        ..of(1)
    };
    let not_draft = BoardRuleError::PlanNotDraft(PlanState::Superseded);
    assert_eq!(refused(settled), rule(not_draft));
    let same = PlanEdit {
        handoff: None,
        entry: PlanEntry::Legacy,
        items: Some(plan_of("a", 2, PlanState::Draft).into_fields().items),
        ..of(2)
    };
    assert_eq!(refused(same), rule(BoardRuleError::NoEdit));
    for status in [TaskStatus::Done, TaskStatus::Archived] {
        let store = holding_plans("a", status, vec![plan_of("a", 1, PlanState::Draft)]);
        let edit = PlanEdit {
            handoff: None,
            entry: PlanEntry::Legacy,
            items: items.clone(),
            ..of(1)
        };
        assert_eq!(
            edit_as(&store, "Ada").execute(&edit),
            Err(ProjectBoardError::Rule(BoardRuleError::TaskFinished(
                status
            )))
        );
    }
}

/// An edit pinned to the draft it read that loses the race to another
/// edit is refused on its second attempt, never written over the other.
#[test]
fn a_pinned_edit_that_loses_the_race_is_refused_as_plan_changed() {
    let store = holding_plans(
        "a",
        TaskStatus::Ready,
        vec![plan_of("a", 1, PlanState::Draft)],
    );
    let other = store.client();
    store.after_next_read(move |_| {
        let theirs = PlanEdit {
            handoff: None,
            entry: PlanEntry::Legacy,
            approach: Some("Theirs.".into()),
            ..of(1)
        };
        edit_as(&other, "Bob").execute(&theirs).unwrap();
    });
    let mine = PlanEdit {
        handoff: None,
        entry: PlanEntry::Legacy,
        items: Some(vec![plan_item("mine", &[])]),
        expected_updated: Some(at("09:00:00")),
        ..of(1)
    };
    let mut facts = OpFacts::default();
    assert_eq!(
        edit_as(&store, "Cy").execute_noted(&mine, Some(&mut facts)),
        Err(ProjectBoardError::Rule(BoardRuleError::PlanChanged {
            version: 1,
            updated: at("10:00:00"),
        }))
    );
    assert_eq!((facts.attempts, facts.lost), (Some(2), Some(1)));
    assert_eq!(store.read().unwrap().plans[0].fields().approach, "Theirs.");
}

/// A set-aside task file holds an edit, as it holds every change.
#[test]
fn a_task_set_aside_holds_edits() {
    let store = holding_plans(
        "a",
        TaskStatus::Ready,
        vec![plan_of("a", 1, PlanState::Draft)],
    );
    store.quarantine("a", "unreadable");
    let edit = PlanEdit {
        handoff: None,
        entry: PlanEntry::Legacy,
        approach: Some("New.".into()),
        ..of(1)
    };
    assert_eq!(
        edit_as(&store, "Ada").execute(&edit),
        Err(crate::application::project_board::use_cases::fixtures_tests::set_aside_task("a"))
    );
}

/// An edit is judged on the draft it writes: a team given, or the draft's
/// kept, whose coordinator role's model names no provider is refused, as
/// a start would refuse it; with its provider, or no team, it is written.
#[test]
fn a_coordinator_model_without_its_provider_is_refused_at_edit() {
    let store = holding_plans(
        "a",
        TaskStatus::Ready,
        vec![plan_of("a", 1, PlanState::Draft)],
    );
    let writes = store.writes();
    let bare = PlanEdit {
        handoff: None,
        entry: PlanEntry::Legacy,
        team: Some(Some(coordinated_team("claude-opus"))),
        ..of(1)
    };
    assert_eq!(
        edit_as(&store, "Ada").execute(&bare),
        Err(unqualified_coordinator())
    );
    assert_eq!(store.writes(), writes, "nothing written");
    let named = coordinated_team("anthropic/claude-opus");
    let qualified = PlanEdit {
        handoff: None,
        entry: PlanEntry::Legacy,
        team: Some(Some(named.clone())),
        ..of(1)
    };
    let edited = edit_as(&store, "Ada").execute(&qualified).unwrap();
    assert_eq!(edited.fields().team, Some(named));
    let none = PlanEdit {
        handoff: None,
        entry: PlanEntry::Legacy,
        team: Some(None),
        ..of(1)
    };
    let edited = edit_as(&store, "Ada").execute(&none).unwrap();
    assert_eq!(edited.fields().team, None);
    let held = Plan::new(PlanFields {
        team: Some(coordinated_team("claude-opus")),
        ..plan_of("a", 1, PlanState::Draft).into_fields()
    })
    .unwrap();
    let store = holding_plans("a", TaskStatus::Ready, vec![held]);
    let approach = PlanEdit {
        handoff: None,
        entry: PlanEntry::Legacy,
        approach: Some("New.".into()),
        ..of(1)
    };
    assert_eq!(
        edit_as(&store, "Ada").execute(&approach),
        Err(unqualified_coordinator()),
        "the kept team is judged too"
    );
}

/// Review round 2 (#2566): an edit judges the model of every coordinator
/// role, whatever its members.
#[test]
fn a_bare_coordinator_model_is_refused_at_edit_whatever_its_members() {
    let store = holding_plans(
        "a",
        TaskStatus::Ready,
        vec![plan_of("a", 1, PlanState::Draft)],
    );
    let mut team = coordinated_team("claude-opus");
    team.roles[0].limits.members = 2;
    let pair = PlanEdit {
        handoff: None,
        entry: PlanEntry::Legacy,
        team: Some(Some(team)),
        ..of(1)
    };
    assert_eq!(
        edit_as(&store, "Ada").execute(&pair),
        Err(unqualified_coordinator())
    );
}
