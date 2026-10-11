use super::*;
use crate::domain::project_board::entities::run::RunFields;
use crate::domain::project_board::services::fixtures_tests::{at, open_run, person, plan, task};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::vocabulary::{RunOutcome, TaskStatus::*};

fn content() -> PlanContent {
    let draft = plan(1, PlanState::Draft).into_fields();
    PlanContent {
        approach: draft.approach,
        items: draft.items,
        team: draft.team,
    }
}

/// `task` in `status` naming no accepted plan.
fn unaccepted(status: TaskStatus) -> Task {
    let mut fields = task(status).into_fields();
    fields.accepted_plan = None;
    Task::new(fields).unwrap()
}

fn other_tasks(version: u32) -> Plan {
    let mut fields = plan(version, PlanState::Draft).into_fields();
    fields.task = Slug::parse("other").unwrap();
    Plan::new(fields).unwrap()
}

#[test]
fn a_new_version_follows_the_last_and_records_who_proposed_it() {
    let versions = [plan(1, PlanState::Accepted), plan(2, PlanState::Draft)];
    let next = propose(
        &task(Ready),
        &versions,
        content(),
        &person("Bob"),
        &at("10:00:00"),
    )
    .unwrap();
    let fields = next.fields();
    assert_eq!(
        (fields.version, fields.state, &fields.created),
        (3, PlanState::Draft, &at("10:00:00"))
    );
    assert_eq!(
        (&fields.proposed_by, &fields.accepted),
        (&person("Bob"), &None)
    );
    let first = propose(
        &unaccepted(Draft),
        &[],
        content(),
        &person("Bob"),
        &at("10:00:00"),
    )
    .unwrap();
    assert_eq!(first.fields().version, 1);
}

#[test]
fn a_finished_tasks_plans_are_settled() {
    let ada = person("Ada");
    let now = at("10:00:00");
    for status in [Done, Archived] {
        let finished = task(status);
        let refused = BoardRuleError::TaskFinished(status);
        assert_eq!(
            propose(
                &finished,
                &[plan(1, PlanState::Accepted)],
                content(),
                &ada,
                &now
            ),
            Err(refused.clone())
        );
        assert_eq!(
            edit_draft(&finished, &plan(2, PlanState::Draft), content(), &now),
            Err(refused.clone())
        );
        assert_eq!(
            discard(&finished, &plan(2, PlanState::Draft), &ada, &now),
            Err(refused.clone())
        );
        let versions = [plan(1, PlanState::Accepted), plan(2, PlanState::Draft)];
        assert_eq!(
            accept(
                &TaskPlans {
                    task: &finished,
                    versions: &versions,
                    open: None
                },
                2,
                None,
                &ada,
                &now
            )
            .map(|a| a.plan),
            Err(refused)
        );
    }
}

/// The rules refuse plans that contradict each other or the task, rather
/// than build on them: a version listed twice, two accepted, or an
/// accepted plan the task does not name.
#[test]
fn plans_that_contradict_each_other_are_refused_not_built_on() {
    let ada = person("Ada");
    let now = at("10:00:00");
    let held = task(InProgress);
    for versions in [
        vec![plan(1, PlanState::Accepted), plan(1, PlanState::Draft)],
        vec![plan(1, PlanState::Accepted), plan(2, PlanState::Accepted)],
        vec![plan(2, PlanState::Accepted)],
        vec![plan(1, PlanState::Draft)],
        vec![plan(1, PlanState::Accepted), other_tasks(2)],
    ] {
        let refused = accept(
            &TaskPlans {
                task: &held,
                versions: &versions,
                open: None,
            },
            1,
            None,
            &ada,
            &now,
        )
        .map(|a| a.plan);
        assert!(
            matches!(
                refused,
                Err(BoardRuleError::PlansInconsistent(_) | BoardRuleError::NotThisTasks)
            ),
            "{versions:?}: {refused:?}"
        );
        assert!(propose(&held, &versions, content(), &ada, &now).is_err());
    }
}

#[test]
fn only_a_draft_is_edited_and_only_by_an_edit_that_changes_something() {
    let draft = plan(2, PlanState::Draft);
    let mut changed = content();
    changed.team = None;
    let held = task(InProgress);
    let edited = edit_draft(&held, &draft, changed.clone(), &at("08:30:00")).unwrap();
    assert_eq!(edited.fields().team, None);
    assert_eq!(edited.fields().updated, at("08:30:01"), "always moves on");
    assert_eq!(
        edit_draft(&held, &draft, content(), &at("10:00:00")),
        Err(BoardRuleError::NoEdit)
    );
    for state in [
        PlanState::Accepted,
        PlanState::Superseded,
        PlanState::Discarded,
    ] {
        assert_eq!(
            edit_draft(&held, &plan(2, state), changed.clone(), &at("10:00:00")),
            Err(BoardRuleError::PlanNotDraft(state))
        );
    }
    assert_eq!(
        edit_draft(&held, &other_tasks(2), changed, &at("10:00:00")),
        Err(BoardRuleError::NotThisTasks)
    );
}

#[test]
fn accepting_names_the_version_records_who_and_supersedes_the_one_before() {
    let versions = [plan(1, PlanState::Accepted), plan(2, PlanState::Draft)];
    let accepted = accept(
        &TaskPlans {
            task: &task(InProgress),
            versions: &versions,
            open: None,
        },
        2,
        None,
        &person("Ada"),
        &at("10:00:00"),
    )
    .unwrap();
    assert_eq!(accepted.task.fields().accepted_plan, Some(2));
    assert_eq!(accepted.task.fields().updated, at("10:00:00"));
    let plan_fields = accepted.plan.fields();
    assert_eq!(
        (plan_fields.state, plan_fields.version),
        (PlanState::Accepted, 2)
    );
    assert_eq!(plan_fields.accepted, Some((person("Ada"), at("10:00:00"))));
    let superseded = accepted.superseded.unwrap();
    assert_eq!(
        (superseded.fields().version, superseded.fields().state),
        (1, PlanState::Superseded)
    );
    assert!(
        superseded.fields().accepted.is_some(),
        "it keeps who accepted it"
    );
    let first = accept(
        &TaskPlans {
            task: &unaccepted(Draft),
            versions: &[plan(1, PlanState::Draft)],
            open: None,
        },
        1,
        None,
        &person("Bob"),
        &at("10:00:00"),
    )
    .unwrap();
    assert_eq!(first.superseded, None, "nothing was accepted before");
}

/// Accepting an older draft over a newer accepted version is a rollback:
/// it is allowed, and supersedes the newer one.
#[test]
fn an_older_draft_accepted_over_a_newer_version_is_a_rollback() {
    let mut fields = task(InProgress).into_fields();
    fields.accepted_plan = Some(2);
    let held = Task::new(fields).unwrap();
    let versions = [plan(1, PlanState::Draft), plan(2, PlanState::Accepted)];
    let rolled = accept(
        &TaskPlans {
            task: &held,
            versions: &versions,
            open: None,
        },
        1,
        None,
        &person("Ada"),
        &at("10:00:00"),
    )
    .unwrap();
    assert_eq!(rolled.task.fields().accepted_plan, Some(1));
    assert_eq!(rolled.superseded.unwrap().fields().version, 2);
}

#[test]
fn the_holder_decides_for_a_held_task_and_anyone_for_an_unclaimed_one() {
    let versions = [plan(1, PlanState::Accepted), plan(2, PlanState::Draft)];
    for status in [Claimed, InProgress, Review, Blocked] {
        let held = task(status);
        assert_eq!(
            accept(
                &TaskPlans {
                    task: &held,
                    versions: &versions,
                    open: None
                },
                2,
                None,
                &person("Bob"),
                &at("10:00:00")
            )
            .map(|accepted| accepted.plan),
            Err(BoardRuleError::NotHolder),
            "{status:?}"
        );
        assert_eq!(
            accept(
                &TaskPlans {
                    task: &held,
                    versions: &versions,
                    open: None
                },
                2,
                None,
                &person("Ada"),
                &at("11:00:00")
            )
            .map(|accepted| accepted.plan),
            Err(BoardRuleError::ClaimExpired),
            "{status:?}: renew first"
        );
        assert_eq!(
            discard(&held, &versions[1], &person("Bob"), &at("10:00:00")),
            Err(BoardRuleError::NotHolder)
        );
    }
    for status in [Draft, Ready] {
        assert!(
            accept(
                &TaskPlans {
                    task: &task(status),
                    versions: &versions,
                    open: None
                },
                2,
                None,
                &person("Bob"),
                &at("10:00:00")
            )
            .is_ok(),
            "{status:?}"
        );
    }
}

#[test]
fn an_acceptance_waits_for_a_known_unchanged_draft_and_no_run_going() {
    let held = task(InProgress);
    let ada = person("Ada");
    let now = at("10:00:00");
    let versions = [plan(1, PlanState::Accepted), plan(2, PlanState::Draft)];
    let accept_with = |version: u32, expected: Option<&Timestamp>, open: Option<&Run>| {
        accept(
            &TaskPlans {
                task: &held,
                versions: &versions,
                open,
            },
            version,
            expected,
            &ada,
            &now,
        )
        .map(|a| a.plan)
    };
    assert_eq!(
        accept_with(1, None, None),
        Err(BoardRuleError::PlanNotDraft(PlanState::Accepted))
    );
    assert_eq!(
        accept_with(3, None, None),
        Err(BoardRuleError::UnknownPlanVersion(3))
    );
    assert_eq!(
        accept_with(2, Some(&at("08:00:00")), None),
        Err(BoardRuleError::PlanChanged {
            version: 2,
            updated: at("08:30:00"),
        })
    );
    assert!(
        accept_with(2, Some(&at("08:30:00")), None).is_ok(),
        "as read"
    );
    let running = open_run(1, 1);
    assert_eq!(
        accept_with(2, None, Some(&running)),
        Err(BoardRuleError::RunOpen(1))
    );
    let mut ended = running.into_fields();
    ended.ended = Some(at("09:30:00"));
    ended.outcome = Some(RunOutcome::Succeeded);
    let ended = Run::new(ended).unwrap();
    assert!(
        accept_with(2, None, Some(&ended)).is_ok(),
        "a run that ended"
    );
    let elsewhere = Run::new(RunFields {
        task: Slug::parse("other").unwrap(),
        ..ended.into_fields()
    })
    .unwrap();
    assert_eq!(
        accept_with(2, None, Some(&elsewhere)),
        Err(BoardRuleError::NotThisTasks)
    );
}

#[test]
fn a_discarded_draft_is_kept_and_only_a_draft_is_discarded() {
    let discarded = discard(
        &task(Ready),
        &plan(2, PlanState::Draft),
        &person("Bob"),
        &at("10:00:00"),
    )
    .unwrap();
    assert_eq!(discarded.fields().state, PlanState::Discarded);
    assert_eq!(
        discarded.fields().items,
        plan(2, PlanState::Draft).fields().items
    );
    assert_eq!(
        discard(&task(Ready), &discarded, &person("Bob"), &at("10:00:00")),
        Err(BoardRuleError::PlanNotDraft(PlanState::Discarded))
    );
}
