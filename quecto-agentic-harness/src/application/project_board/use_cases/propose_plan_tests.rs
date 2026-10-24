//! #2481 M4b: a proposal writes a new draft version alone, its text
//! normalised as it enters.
use super::ProposePlan;
use crate::application::project_board::dto::{PlanProposal, ProjectBoardError};
use crate::application::project_board::ports::{ArtifactKey, ProjectBoardStore};
use crate::application::project_board::telemetry::OpFacts;
use crate::application::project_board::use_cases::fixtures_tests::{
    SetClock, at, coordinated_team, holding_plans, person, plan_item, plan_of, plan_team, slug,
    unqualified_coordinator,
};
use crate::domain::project_board::entities::plan::PlanEntry;
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::telemetry::{Normalised, TextChange};
use crate::domain::project_board::value_objects::vocabulary::{PlanState, TaskStatus};
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use std::sync::Arc;

fn propose(store: &MemoryBoardStore) -> ProposePlan {
    ProposePlan::new(Arc::new(store.client()), SetClock::at("10:00:00")).unwrap()
}

fn proposal(task: &str) -> PlanProposal {
    PlanProposal {
        handoff: None,
        entry: PlanEntry::Legacy,
        task: slug(task),
        approach: "Build it.".into(),
        items: vec![plan_item("schema", &[]), plan_item("api", &["schema"])],
        team: Some(plan_team()),
    }
}

/// A plan-only commit: the new version follows the last, by the store's
/// committer, and the task's file is left exactly as it was.
#[test]
fn a_proposal_writes_only_its_plan_as_the_next_version() {
    let store = holding_plans(
        "a",
        TaskStatus::Ready,
        vec![plan_of("a", 1, PlanState::Draft)],
    );
    let before = store.read().unwrap();
    let written = propose(&store).execute(&proposal("a")).unwrap();
    let plan = written.fields();
    assert_eq!(
        (plan.version, plan.state, &plan.proposed_by, &plan.edited_by),
        (2, PlanState::Draft, &person("Ada"), &None)
    );
    assert_eq!(
        (&plan.created, &plan.updated),
        (&at("10:00:00"), &at("10:00:00"))
    );
    let after = store.read().unwrap();
    assert_ne!(after.head, before.head, "one write");
    assert_eq!(after.tasks, before.tasks, "the task is untouched");
    assert_eq!(
        after.plans,
        vec![plan_of("a", 1, PlanState::Draft), written]
    );
    assert_eq!(
        store.message_at(&after.head).as_deref(),
        Some("board: propose a plan for a")
    );
    let first = holding_plans("b", TaskStatus::Draft, vec![]);
    let written = propose(&first).execute(&proposal("b")).unwrap();
    assert_eq!(written.fields().version, 1);
}

/// CRLF and other spacing in the plan's text are normalised as it enters,
/// and what changed is noted by kind.
#[test]
fn crlf_in_the_approach_is_normalised_and_noted() {
    let store = holding_plans("a", TaskStatus::Ready, vec![]);
    let mut raw = proposal("a");
    raw.approach = "Spike first.\r\nThen build. \r\n".into();
    raw.items[0].title = " Item schema".into();
    raw.items[1].criteria = vec!["api\tproved".into()];
    let mut facts = OpFacts::default();
    let written = propose(&store)
        .execute_noted(&raw, Some(&mut facts))
        .unwrap();
    let plan = written.fields();
    assert_eq!(plan.approach, "Spike first.\nThen build.");
    assert_eq!(
        (
            plan.items[0].title.as_str(),
            plan.items[1].criteria[0].as_str()
        ),
        ("Item schema", "api proved")
    );
    let want = OpFacts {
        attempts: Some(1),
        lost: Some(0),
        normalised: Some(Normalised {
            title: None,
            description: None,
            approach: Some(vec![
                TextChange::Crlf,
                TextChange::EdgeBlankLines,
                TextChange::TrailingSpace,
            ]),
            items: Some(vec![TextChange::Tab, TextChange::EdgeSpace]),
        }),
        plan_version: Some(1),
        items: Some(2),
        roles: Some(1),
        // The commit the write made (#2663).
        head: Some(crate::application::project_board::ports::BoardHead(
            "r3".into(),
        )),
        ..OpFacts::default()
    };
    assert_eq!(facts, want);
}

#[test]
fn a_done_or_archived_tasks_plans_are_settled() {
    for status in [TaskStatus::Done, TaskStatus::Archived] {
        let store = holding_plans("a", status, vec![]);
        let writes = store.writes();
        let mut facts = OpFacts::default();
        assert_eq!(
            propose(&store).execute_noted(&proposal("a"), Some(&mut facts)),
            Err(ProjectBoardError::Rule(BoardRuleError::TaskFinished(
                status
            )))
        );
        assert_eq!(store.writes(), writes, "nothing written");
        assert_eq!((facts.plan_version, facts.items), (None, None));
    }
}

/// While a plan of the task is set aside, the next number may be its
/// file's: nothing is proposed until it is mended or cleared.
#[test]
fn a_plan_set_aside_holds_proposals_and_a_missing_task_is_refused() {
    let store = holding_plans(
        "a",
        TaskStatus::Ready,
        vec![plan_of("a", 1, PlanState::Draft)],
    );
    let key = ArtifactKey::Plan(slug("a"), 2);
    store.quarantine_key(key.clone(), "plans/a.2.md", "unreadable");
    assert_eq!(
        propose(&store).execute(&proposal("a")),
        Err(ProjectBoardError::Quarantined {
            key,
            path: "plans/a.2.md".into(),
            object: crate::infrastructure::test_support::memory_board_store::memory_object(
                "plans/a.2.md"
            ),
        })
    );
    assert_eq!(
        propose(&store).execute(&proposal("missing")),
        Err(ProjectBoardError::NotFound(slug("missing")))
    );
}

/// A proposal that loses the race to a rival's version 2 is rebuilt
/// against the fresh board and writes version 3, the rival's kept.
#[test]
fn a_proposal_that_loses_to_a_rivals_version_writes_the_next() {
    let store = holding_plans(
        "a",
        TaskStatus::Ready,
        vec![plan_of("a", 1, PlanState::Draft)],
    );
    let rival = store.client();
    store.after_next_read(move |_| {
        propose(&rival).execute(&proposal("a")).unwrap();
    });
    let mut facts = OpFacts::default();
    let written = propose(&store)
        .execute_noted(&proposal("a"), Some(&mut facts))
        .unwrap();
    assert_eq!(written.fields().version, 3);
    let versions: Vec<u32> = store
        .read()
        .unwrap()
        .plans
        .iter()
        .map(|plan| plan.fields().version)
        .collect();
    assert_eq!(versions, [1, 2, 3]);
    assert_eq!(
        (facts.attempts, facts.lost, facts.plan_version),
        (Some(2), Some(1), Some(3))
    );
}

/// A set-aside task file holds a proposal, as it holds every change.
#[test]
fn a_task_set_aside_holds_proposals() {
    let store = holding_plans("a", TaskStatus::Ready, vec![]);
    store.quarantine("a", "unreadable");
    assert_eq!(
        propose(&store).execute(&proposal("a")),
        Err(crate::application::project_board::use_cases::fixtures_tests::set_aside_task("a"))
    );
}

/// A coordinator role's model names its provider, as a start needs it, so
/// the planner learns it when proposing, not when the user starts the
/// task; a team with no coordinator role, or none at all, lets the
/// coordinator be chosen.
#[test]
fn a_coordinator_model_without_its_provider_is_refused_at_proposal() {
    for bare in ["claude-opus", "gpt-5.5"] {
        let store = holding_plans("a", TaskStatus::Ready, vec![]);
        let writes = store.writes();
        let mut facts = OpFacts::default();
        let unqualified = PlanProposal {
            handoff: None,
            entry: PlanEntry::Legacy,
            team: Some(coordinated_team(bare)),
            ..proposal("a")
        };
        assert_eq!(
            propose(&store).execute_noted(&unqualified, Some(&mut facts)),
            Err(unqualified_coordinator()),
            "{bare}"
        );
        assert_eq!(store.writes(), writes, "nothing written for {bare}");
        assert_eq!((facts.plan_version, facts.roles), (None, None));
    }
    for team in [
        Some(coordinated_team("anthropic/claude-opus")),
        Some(plan_team()),
        None,
    ] {
        let store = holding_plans("a", TaskStatus::Ready, vec![]);
        let named = PlanProposal {
            handoff: None,
            entry: PlanEntry::Legacy,
            team: team.clone(),
            ..proposal("a")
        };
        let written = propose(&store).execute(&named).unwrap();
        assert_eq!(written.fields().team, team);
    }
}

/// Review round 2 (#2566): the model of every coordinator role is judged,
/// whatever its members, so a bare model on a coordinator role of two is
/// refused here, where a start would refuse the members first.
#[test]
fn a_bare_coordinator_model_is_refused_whatever_its_members() {
    let store = holding_plans("a", TaskStatus::Ready, vec![]);
    let mut team = coordinated_team("claude-opus");
    team.roles[0].limits.members = 2;
    let pair = PlanProposal {
        handoff: None,
        entry: PlanEntry::Legacy,
        team: Some(team),
        ..proposal("a")
    };
    assert_eq!(
        propose(&store).execute(&pair),
        Err(unqualified_coordinator())
    );
}
