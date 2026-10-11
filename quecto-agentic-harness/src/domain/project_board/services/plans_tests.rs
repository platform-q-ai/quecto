use super::*;
use crate::domain::project_board::services::fixtures_tests::{at, open_run, person, plan, task};
use crate::domain::project_board::value_objects::vocabulary::TaskStatus::*;

fn content() -> PlanContent {
    let draft = plan(1, PlanState::Draft).into_fields();
    PlanContent {
        approach: draft.approach,
        items: draft.items,
        team: draft.team,
    }
}

#[test]
fn a_new_version_follows_the_last_and_starts_as_a_draft() {
    let versions = [plan(1, PlanState::Superseded), plan(2, PlanState::Accepted)];
    let next = propose(&task(Ready), &versions, content(), &at("10:00:00")).unwrap();
    let fields = next.fields();
    assert_eq!(
        (fields.version, fields.state, &fields.created),
        (3, PlanState::Draft, &at("10:00:00"))
    );
    let first = propose(&task(Draft), &[], content(), &at("10:00:00")).unwrap();
    assert_eq!(first.fields().version, 1);
}

#[test]
fn a_finished_task_takes_no_plan_and_versions_are_the_tasks_own() {
    for status in [Done, Archived] {
        assert_eq!(
            propose(&task(status), &[], content(), &at("10:00:00")),
            Err(BoardRuleError::TaskFinished(status))
        );
    }
    let mut other = plan(1, PlanState::Draft).into_fields();
    other.task = crate::domain::project_board::value_objects::slug::Slug::parse("other").unwrap();
    let other = Plan::new(other).unwrap();
    assert_eq!(
        propose(&task(Ready), &[other], content(), &at("10:00:00")),
        Err(BoardRuleError::NotThisTasks)
    );
    let mut empty = content();
    empty.approach = "\u{0}".into();
    assert!(matches!(
        propose(&task(Ready), &[], empty, &at("10:00:00")),
        Err(BoardRuleError::Invalid(_))
    ));
}

#[test]
fn only_a_draft_is_edited_and_only_by_an_edit_that_changes_something() {
    let draft = plan(1, PlanState::Draft);
    let mut changed = content();
    changed.team = None;
    let edited = edit_draft(&draft, changed.clone(), &at("08:30:00")).unwrap();
    assert_eq!(edited.fields().team, None);
    assert_eq!(edited.fields().updated, at("08:30:01"), "always moves on");
    assert_eq!(
        edit_draft(&draft, content(), &at("10:00:00")),
        Err(BoardRuleError::NoEdit)
    );
    for state in [
        PlanState::Accepted,
        PlanState::Superseded,
        PlanState::Discarded,
    ] {
        assert_eq!(
            edit_draft(&plan(1, state), changed.clone(), &at("10:00:00")),
            Err(BoardRuleError::PlanNotDraft(state))
        );
    }
}

#[test]
fn accepting_names_the_version_and_supersedes_the_one_before() {
    let versions = [plan(1, PlanState::Accepted), plan(2, PlanState::Draft)];
    let accepted = accept(
        &task(InProgress),
        &versions,
        2,
        &person("Ada"),
        None,
        &at("10:00:00"),
    )
    .unwrap();
    assert_eq!(accepted.task.fields().accepted_plan, Some(2));
    assert_eq!(accepted.task.fields().updated, at("10:00:00"));
    assert_eq!(accepted.plan.fields().state, PlanState::Accepted);
    assert_eq!(accepted.plan.fields().version, 2);
    let superseded = accepted.superseded.unwrap();
    assert_eq!(
        (superseded.fields().version, superseded.fields().state),
        (1, PlanState::Superseded)
    );
    let first = accept(
        &task(Draft),
        &[plan(1, PlanState::Draft)],
        1,
        &person("Bob"),
        None,
        &at("10:00:00"),
    )
    .unwrap();
    assert_eq!(first.superseded, None, "nothing was accepted before");
}

#[test]
fn the_holder_decides_for_a_held_task_and_anyone_for_an_unclaimed_one() {
    let versions = [plan(1, PlanState::Draft)];
    for status in [Claimed, InProgress, Review, Blocked] {
        assert_eq!(
            accept(
                &task(status),
                &versions,
                1,
                &person("Bob"),
                None,
                &at("10:00:00")
            )
            .map(|accepted| accepted.plan),
            Err(BoardRuleError::NotHolder),
            "{status:?}"
        );
        assert_eq!(
            accept(
                &task(status),
                &versions,
                1,
                &person("Ada"),
                None,
                &at("11:00:00")
            )
            .map(|accepted| accepted.plan),
            Err(BoardRuleError::ClaimExpired),
            "{status:?}: renew first"
        );
        assert_eq!(
            discard(&task(status), &versions[0], &person("Bob"), &at("10:00:00")),
            Err(BoardRuleError::NotHolder)
        );
    }
    for status in [Draft, Ready] {
        assert!(
            accept(
                &task(status),
                &versions,
                1,
                &person("Bob"),
                None,
                &at("10:00:00")
            )
            .is_ok(),
            "{status:?}"
        );
    }
}

#[test]
fn an_acceptance_waits_for_a_draft_a_known_version_and_no_run_going() {
    let held = task(InProgress);
    let ada = person("Ada");
    let now = at("10:00:00");
    assert_eq!(
        accept(&held, &[plan(1, PlanState::Accepted)], 1, &ada, None, &now).map(|a| a.plan),
        Err(BoardRuleError::PlanNotDraft(PlanState::Accepted))
    );
    assert_eq!(
        accept(&held, &[plan(1, PlanState::Draft)], 2, &ada, None, &now).map(|a| a.plan),
        Err(BoardRuleError::NotThisTasks)
    );
    let running = open_run(1, 1);
    assert_eq!(
        accept(
            &held,
            &[plan(2, PlanState::Draft)],
            2,
            &ada,
            Some(&running),
            &now
        )
        .map(|a| a.plan),
        Err(BoardRuleError::RunOpen(1))
    );
    for status in [Done, Archived] {
        assert_eq!(
            accept(
                &task(status),
                &[plan(1, PlanState::Draft)],
                1,
                &ada,
                None,
                &now
            )
            .map(|a| a.plan),
            Err(BoardRuleError::TaskFinished(status))
        );
    }
}

#[test]
fn a_discarded_draft_is_kept_and_only_a_draft_is_discarded() {
    let discarded = discard(
        &task(Ready),
        &plan(1, PlanState::Draft),
        &person("Bob"),
        &at("10:00:00"),
    )
    .unwrap();
    assert_eq!(discarded.fields().state, PlanState::Discarded);
    assert_eq!(
        discarded.fields().items,
        plan(1, PlanState::Draft).fields().items
    );
    assert_eq!(
        discard(&task(Ready), &discarded, &person("Bob"), &at("10:00:00")),
        Err(BoardRuleError::PlanNotDraft(PlanState::Discarded))
    );
}
