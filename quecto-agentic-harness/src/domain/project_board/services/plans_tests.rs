use super::*;
use crate::domain::project_board::entities::run::RunFields;
use crate::domain::project_board::services::fixtures_tests::{at, open_run, person, plan, task};
use crate::domain::project_board::value_objects::board_text;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::vocabulary::{RunOutcome, TaskStatus::*};

/// `now`, with no outside bound.
fn when(now: &Timestamp) -> Moment {
    Moment {
        now: now.clone(),
        latest: None,
    }
}

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
    let next = propose_at(
        &task(Ready),
        &versions,
        content(),
        &person("Bob"),
        &when(&at("10:00:00")),
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
    let first = propose_at(
        &unaccepted(Draft),
        &[],
        content(),
        &person("Bob"),
        &when(&at("10:00:00")),
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
            propose_at(
                &finished,
                &[plan(1, PlanState::Accepted)],
                content(),
                &ada,
                &when(&now.clone())
            ),
            Err(refused.clone())
        );
        assert_eq!(
            edit_draft(
                DraftOf {
                    task: &finished,
                    versions: std::slice::from_ref(&plan(2, PlanState::Draft)),
                    plan: &plan(2, PlanState::Draft)
                },
                content(),
                None,
                &person("Ada"),
                &when(&now)
            ),
            Err(refused.clone())
        );
        assert_eq!(
            discard(
                DraftOf {
                    task: &finished,
                    versions: std::slice::from_ref(&plan(2, PlanState::Draft)),
                    plan: &plan(2, PlanState::Draft)
                },
                None,
                &ada,
                &when(&now)
            ),
            Err(refused.clone())
        );
        let versions = [plan(1, PlanState::Accepted), plan(2, PlanState::Draft)];
        assert_eq!(
            accept(
                &TaskPlans {
                    task: &finished,
                    versions: &versions,
                    runs: &[]
                },
                2,
                None,
                &ada,
                &when(&now)
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
    let two = accept(
        &TaskPlans {
            task: &held,
            versions: &[plan(1, PlanState::Accepted), plan(2, PlanState::Accepted)],
            runs: &[],
        },
        1,
        None,
        &ada,
        &when(&now),
    )
    .map(|accepted| accepted.plan);
    assert_eq!(
        two,
        Err(BoardRuleError::PlansInconsistent(
            "versions 1, 2 are accepted; the task names version 1".into()
        ))
    );
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
                runs: &[],
            },
            1,
            None,
            &ada,
            &when(&now),
        )
        .map(|a| a.plan);
        assert!(
            matches!(
                refused,
                Err(BoardRuleError::PlansInconsistent(_) | BoardRuleError::NotThisTasks)
            ),
            "{versions:?}: {refused:?}"
        );
        assert!(propose_at(&held, &versions, content(), &ada, &when(&now.clone())).is_err());
    }
}

#[test]
fn only_a_draft_is_edited_and_only_by_an_edit_that_changes_something() {
    let draft = plan(2, PlanState::Draft);
    let mut changed = content();
    changed.team = None;
    let held = task(InProgress);
    let edited = edit_draft(
        DraftOf {
            task: &held,
            versions: std::slice::from_ref(&draft),
            plan: &draft,
        },
        changed.clone(),
        None,
        &person("Ada"),
        &when(&at("08:30:00")),
    )
    .unwrap();
    assert_eq!(edited.fields().team, None);
    // Never before the task's last change, at 09:00 (#2662).
    assert_eq!(edited.fields().updated, at("09:00:00"), "always moves on");
    assert_eq!(
        edit_draft(
            DraftOf {
                task: &held,
                versions: std::slice::from_ref(&draft),
                plan: &draft,
            },
            content(),
            None,
            &person("Ada"),
            &when(&at("10:00:00"))
        ),
        Err(BoardRuleError::NoEdit)
    );
    for state in [
        PlanState::Accepted,
        PlanState::Superseded,
        PlanState::Discarded,
    ] {
        assert_eq!(
            edit_draft(
                DraftOf {
                    task: &held,
                    versions: std::slice::from_ref(&plan(2, state)),
                    plan: &plan(2, state)
                },
                changed.clone(),
                None,
                &person("Ada"),
                &when(&at("10:00:00"))
            ),
            Err(BoardRuleError::PlanNotDraft(state))
        );
    }
    assert_eq!(
        edit_draft(
            DraftOf {
                task: &held,
                versions: std::slice::from_ref(&other_tasks(2)),
                plan: &other_tasks(2),
            },
            changed,
            None,
            &person("Ada"),
            &when(&at("10:00:00"))
        ),
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
            runs: &[],
        },
        2,
        None,
        &person("Ada"),
        &when(&at("10:00:00")),
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
            runs: &[],
        },
        1,
        None,
        &person("Bob"),
        &when(&at("10:00:00")),
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
            runs: &[],
        },
        1,
        None,
        &person("Ada"),
        &when(&at("10:00:00")),
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
                    runs: &[]
                },
                2,
                None,
                &person("Bob"),
                &when(&at("10:00:00"))
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
                    runs: &[]
                },
                2,
                None,
                &person("Ada"),
                &when(&at("11:00:00"))
            )
            .map(|accepted| accepted.plan),
            Err(BoardRuleError::ClaimExpired),
            "{status:?}: renew first"
        );
        assert_eq!(
            discard(
                DraftOf {
                    task: &held,
                    versions: std::slice::from_ref(&versions[1]),
                    plan: &versions[1],
                },
                None,
                &person("Bob"),
                &when(&at("10:00:00"))
            ),
            Err(BoardRuleError::NotHolder)
        );
    }
    for status in [Draft, Ready] {
        assert!(
            accept(
                &TaskPlans {
                    task: &task(status),
                    versions: &versions,
                    runs: &[]
                },
                2,
                None,
                &person("Bob"),
                &when(&at("10:00:00"))
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
    let accept_with = |version: u32, expected: Option<&Timestamp>, runs: &[Run]| {
        let accepted = held.fields().accepted_plan;
        let expected = expected.map(|updated| AsRead { updated, accepted });
        accept(
            &TaskPlans {
                task: &held,
                versions: &versions,
                runs,
            },
            version,
            expected,
            &ada,
            &when(&now),
        )
        .map(|a| a.plan)
    };
    assert_eq!(
        accept_with(1, None, &[]),
        Err(BoardRuleError::PlanNotDraft(PlanState::Accepted))
    );
    assert_eq!(
        accept_with(3, None, &[]),
        Err(BoardRuleError::UnknownPlanVersion(3))
    );
    assert_eq!(
        accept_with(2, Some(&at("08:00:00")), &[]),
        Err(BoardRuleError::PlanChanged {
            version: 2,
            updated: at("08:30:00"),
        })
    );
    assert!(
        accept_with(2, Some(&at("08:30:00")), &[]).is_ok(),
        "as read"
    );
    let running = open_run(1, 1);
    assert_eq!(
        accept_with(2, None, std::slice::from_ref(&running)),
        Err(BoardRuleError::RunOpen(1))
    );
    let mut ended = running.into_fields();
    ended.ended = Some(at("09:30:00"));
    ended.outcome = Some(RunOutcome::Succeeded);
    let ended = Run::new(ended).unwrap();
    assert!(
        accept_with(2, None, std::slice::from_ref(&ended)).is_ok(),
        "a run that ended"
    );
    let elsewhere = Run::new(RunFields {
        task: Slug::parse("other").unwrap(),
        ..ended.into_fields()
    })
    .unwrap();
    assert_eq!(
        accept_with(2, None, std::slice::from_ref(&elsewhere)),
        Err(BoardRuleError::NotThisTasks)
    );
}

/// A plan proposed by a clock running ahead is accepted after its own
/// last change: the task, both plans and the acceptance share one stamp,
/// a second past the latest of them.
#[test]
fn a_plan_from_a_clock_running_ahead_is_accepted_after_it() {
    let mut ahead = plan(2, PlanState::Draft).into_fields();
    (ahead.created, ahead.updated) = (at("12:00:00"), at("12:00:00"));
    let versions = [plan(1, PlanState::Accepted), Plan::new(ahead).unwrap()];
    let accepted = accept(
        &TaskPlans {
            task: &task(InProgress),
            versions: &versions,
            runs: &[],
        },
        2,
        None,
        &person("Ada"),
        &when(&at("10:00:00")),
    )
    .unwrap();
    let stamp = at("12:00:01");
    assert_eq!(accepted.task.fields().updated, stamp);
    assert_eq!(accepted.plan.fields().updated, stamp);
    assert_eq!(
        accepted.plan.fields().accepted,
        Some((person("Ada"), stamp.clone()))
    );
    assert_eq!(accepted.superseded.unwrap().fields().updated, stamp);
}

/// Runs that contradict each other (two going) are refused, never chosen
/// between.
#[test]
fn runs_that_contradict_each_other_are_refused() {
    let both = [open_run(1, 1), open_run(2, 1)];
    let versions = [plan(1, PlanState::Accepted), plan(2, PlanState::Draft)];
    let refused = accept(
        &TaskPlans {
            task: &task(InProgress),
            versions: &versions,
            runs: &both,
        },
        2,
        None,
        &person("Ada"),
        &when(&at("10:00:00")),
    );
    assert!(
        matches!(&refused, Err(BoardRuleError::RunsInconsistent(why)) if why.contains("1, 2")),
        "{refused:?}"
    );
}

/// An edit of a draft can be pinned to the draft as read, so a newer edit
/// is never overwritten.
#[test]
fn a_draft_edit_pinned_to_an_older_read_is_refused() {
    let mut changed = content();
    changed.team = None;
    let draft = plan(2, PlanState::Draft);
    assert_eq!(
        edit_draft(
            DraftOf {
                task: &task(InProgress),
                versions: std::slice::from_ref(&draft),
                plan: &draft,
            },
            changed.clone(),
            Some(&at("08:00:00")),
            &person("Ada"),
            &when(&at("10:00:00"))
        ),
        Err(BoardRuleError::PlanChanged {
            version: 2,
            updated: at("08:30:00"),
        })
    );
    assert!(
        edit_draft(
            DraftOf {
                task: &task(InProgress),
                versions: std::slice::from_ref(&draft),
                plan: &draft,
            },
            changed,
            Some(&at("08:30:00")),
            &person("Ada"),
            &when(&at("10:00:00"))
        )
        .is_ok()
    );
}

/// A plan stamped by a clock that ran far ahead is never built on: an
/// acceptance or an edit with an outside bound refuses it, as the task's
/// own changes refuse a task stamped ahead. The bound covers the plan
/// accepted and the plan it would supersede.
#[test]
fn a_plan_stamped_ahead_of_the_outside_time_is_never_built_on() {
    let far = Timestamp::parse("9999-01-01T00:00:00Z").unwrap();
    let ahead = |state: PlanState, version: u32| {
        let mut fields = plan(version, state).into_fields();
        (fields.created, fields.updated) = (far.clone(), far.clone());
        if let Some((by, _)) = fields.accepted.clone() {
            fields.accepted = Some((by, far.clone()));
        }
        Plan::new(fields).unwrap()
    };
    let bounded = Moment {
        now: at("10:00:00"),
        latest: Some(at("10:00:00")),
    };
    let held = task(InProgress);
    let ada = person("Ada");
    let accept_in = |versions: &[Plan]| {
        accept(
            &TaskPlans {
                task: &held,
                versions,
                runs: &[],
            },
            2,
            None,
            &ada,
            &bounded,
        )
        .map(|accepted| accepted.plan)
    };
    let chosen = [plan(1, PlanState::Accepted), ahead(PlanState::Draft, 2)];
    assert_eq!(
        accept_in(&chosen),
        Err(BoardRuleError::PlanAhead {
            version: 2,
            updated: far.clone(),
        })
    );
    let superseded = [ahead(PlanState::Accepted, 1), plan(2, PlanState::Draft)];
    assert_eq!(
        accept_in(&superseded),
        Err(BoardRuleError::PlanAhead {
            version: 1,
            updated: far.clone(),
        }),
        "only the plan it would supersede is ahead"
    );
    assert_eq!(
        edit_draft(
            DraftOf {
                task: &held,
                versions: std::slice::from_ref(&ahead(PlanState::Draft, 2)),
                plan: &ahead(PlanState::Draft, 2)
            },
            content(),
            None,
            &ada,
            &bounded
        ),
        Err(BoardRuleError::PlanAhead {
            version: 2,
            updated: far,
        })
    );
    assert!(accept_in(&[plan(1, PlanState::Accepted), plan(2, PlanState::Draft)]).is_ok());
}

/// The bound is the latest time plus the stamp's slack, inclusive: with
/// the latest at 10:00:00, a plan stamped 10:00:30 is accepted and one
/// stamped 10:00:31 is refused.
#[test]
fn the_plan_bound_is_the_latest_time_plus_its_slack() {
    let stamped = |time: &str| {
        let mut fields = plan(2, PlanState::Draft).into_fields();
        (fields.created, fields.updated) = (at(time), at(time));
        Plan::new(fields).unwrap()
    };
    let bounded = Moment {
        now: at("10:00:00"),
        latest: Some(at("10:00:00")),
    };
    let held = task(InProgress);
    let accept_at = |time: &str| {
        let versions = [plan(1, PlanState::Accepted), stamped(time)];
        accept(
            &TaskPlans {
                task: &held,
                versions: &versions,
                runs: &[],
            },
            2,
            None,
            &person("Ada"),
            &bounded,
        )
        .map(|accepted| accepted.plan.fields().version)
    };
    assert_eq!(accept_at("10:00:30"), Ok(2));
    assert_eq!(
        accept_at("10:00:31"),
        Err(BoardRuleError::PlanAhead {
            version: 2,
            updated: at("10:00:31"),
        })
    );
}

/// A change to a plan's content.
type Spoil = fn(&mut PlanContent);

/// A plan's text is taken only as it enters the board, normalised: its
/// approach as markdown, its task items' titles and criteria as lines.
/// Each case is text the plan's schema takes (a trailing space, a leading
/// one), so only the guard refuses it, naming its field; the same text
/// normalised is taken.
#[test]
fn plan_text_is_taken_only_once_normalised() {
    let spoiled: [(&str, Spoil); 3] = [
        ("approach", |c| c.approach = "Spike, then build. ".into()),
        ("items/0/title", |c| {
            c.items[0].title = " Item schema".into()
        }),
        ("items/1/criteria/0", |c| {
            c.items[1].criteria[0] = " api proved".into()
        }),
    ];
    let held = unaccepted(InProgress);
    for (field, spoil) in spoiled {
        let mut raw = content();
        spoil(&mut raw);
        let ada = person("Ada");
        let proposed = propose_at(&held, &[], raw.clone(), &ada, &when(&at("10:00:00")));
        let edited = edit_draft(
            DraftOf {
                task: &held,
                versions: std::slice::from_ref(&plan(1, PlanState::Draft)),
                plan: &plan(1, PlanState::Draft),
            },
            raw,
            None,
            &ada,
            &when(&at("10:00:00")),
        );
        for refused in [proposed, edited] {
            match refused {
                Err(BoardRuleError::InvalidPlan(error)) => assert_eq!(error.field, field),
                other => panic!("{field}: {other:?}"),
            }
        }
    }
    let mut spoilt = content();
    for (_, spoil) in spoiled {
        spoil(&mut spoilt);
    }
    let fields = PlanFields {
        approach: spoilt.approach.clone(),
        items: spoilt.items.clone(),
        ..plan(1, PlanState::Draft).into_fields()
    };
    assert!(Plan::new(fields).is_ok(), "the schema takes every case");
    let mut normalised = content();
    normalised.approach = board_text::text("Spike, then build. ");
    normalised.items[0].title = board_text::line(" Item schema");
    normalised.items[1].criteria[0] = board_text::line(" api proved");
    assert!(
        propose_at(
            &held,
            &[],
            normalised,
            &person("Ada"),
            &when(&at("10:00:00"))
        )
        .is_ok()
    );
}
