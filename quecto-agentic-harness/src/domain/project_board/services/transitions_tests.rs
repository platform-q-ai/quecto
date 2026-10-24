use super::*;
use crate::domain::project_board::entities::claim::{MAX_EMAIL_BYTES, MAX_NAME_CHARS};
use crate::domain::project_board::entities::task::{
    MAX_DESCRIPTION_CHARS, MAX_TEXT_BYTES, MAX_TITLE_CHARS,
};
use crate::domain::project_board::services::fixtures_tests::{
    at, change, commit, far_future, last_written, person, task, without_claim, without_prs,
};
use crate::domain::project_board::value_objects::reference::{MAX_REFERENCE_BYTES, Reference};
use crate::domain::project_board::value_objects::slug::Slug;

const ALL: [TaskStatus; 8] = [
    Draft, Ready, Claimed, InProgress, Review, Blocked, Done, Archived,
];
/// Written out independently of the production table (#2705 T4: the
/// lifecycle makes the rest, `transitions_lifecycle_tests`).
const ALLOWED: [(TaskStatus, TaskStatus); 11] = [
    (Draft, Ready),
    (Draft, Archived),
    (Ready, Draft),
    (Ready, Claimed),
    (Ready, Archived),
    (Claimed, Ready),
    (Review, Done),
    (Review, Ready),
    (Review, Archived),
    (Blocked, Ready),
    (Done, Archived),
];
/// The moves anyone makes, whoever holds a live claim.
const ANYONES: [(TaskStatus, TaskStatus); 6] = [
    (Draft, Ready),
    (Draft, Archived),
    (Ready, Draft),
    (Ready, Claimed),
    (Ready, Archived),
    (Done, Archived),
];
/// The holder's while the claim is live, anyone's once it is not.
const WHILE_LIVE: [(TaskStatus, TaskStatus); 1] = [(Review, Archived)];
/// The moves that hand a task back to the board (#2705 T5: a task in
/// progress goes back by its run's end).
const RELEASES: [(TaskStatus, TaskStatus); 3] =
    [(Claimed, Ready), (Review, Ready), (Blocked, Ready)];

/// `move_with_reason` with no runs, changes or tasks, giving a reason to
/// the one move that takes one (back to ready from review).
fn move_given(
    task: &Task,
    to: TaskStatus,
    actor: &Identity,
    now: &Timestamp,
) -> Result<Task, BoardRuleError> {
    let reason = match (task.fields().status, to) {
        (Review, Ready) => MoveReason::Given("Back for a new plan".into()),
        _ => MoveReason::Absent,
    };
    move_with_reason(task, (to, &reason), actor, (&[], &[]), &[], now)
}

#[test]
fn only_the_table_allows_a_move_even_for_the_holder() {
    for from in ALL {
        for to in ALL {
            let moved = move_given(
                &without_prs(task(from)),
                to,
                &person("Ada"),
                &at("10:00:00"),
            );
            match moved {
                Ok(next) => {
                    assert!(
                        ALLOWED.contains(&(from, to)),
                        "{from:?} -> {to:?} must be refused"
                    );
                    assert_eq!(
                        (next.fields().status, &next.fields().updated),
                        (to, &at("10:00:00"))
                    );
                }
                Err(error) => {
                    assert!(
                        !ALLOWED.contains(&(from, to)),
                        "{from:?} -> {to:?}: {error}"
                    );
                    let refused = match lifecycle_action(from, to) {
                        Some(action) => BoardRuleError::UseLifecycle { from, to, action },
                        None => BoardRuleError::NotAllowed { from, to },
                    };
                    assert_eq!(error, refused);
                }
            }
        }
    }
}

#[test]
fn anyone_plans_readies_claims_and_archives_but_only_the_holder_moves_a_live_claim() {
    let bob = person("Bob");
    for (from, to) in ALLOWED {
        // No pull request linked: a reviewed task linking one is not
        // archived (`transitions_archive_tests`).
        let moved = move_given(&without_prs(task(from)), to, &bob, &at("10:00:00"));
        let anyones = ANYONES.contains(&(from, to));
        assert_eq!(
            moved.is_ok(),
            anyones,
            "{from:?} -> {to:?} by Bob: {moved:?}"
        );
        if !anyones {
            assert_eq!(moved, Err(BoardRuleError::NotHolder));
        }
    }
}

#[test]
fn the_holder_is_known_by_email_whatever_its_case() {
    let shouting = Identity {
        name: "ADA".into(),
        email: "ADA@EXAMPLE.COM".into(),
    };
    assert!(move_given(&without_prs(task(Review)), Done, &shouting, &at("10:00:00")).is_ok());
}

#[test]
fn a_claim_is_taken_kept_or_cleared_by_the_move() {
    let now = at("10:00:00");
    let claimed = move_task(&task(Ready), Claimed, &person("Bob"), (&[], &[]), &[], &now).unwrap();
    let claim = claimed.fields().claim.clone().unwrap();
    assert_eq!(
        (claim.holder, claim.since, claim.expires),
        (person("Bob"), now.clone(), at("12:00:00"))
    );
    // No move keeps a holder's claim since #2705 T4: the moves among the
    // held statuses are the lifecycle's (`start`, `unstart`).
    let cleared = RELEASES.iter().chain(&[(Review, Done), (Review, Archived)]);
    for (from, to) in cleared.copied() {
        let next = move_given(&without_prs(task(from)), to, &person("Ada"), &now).unwrap();
        assert_eq!(next.fields().claim, None, "{from:?} -> {to:?} clears it");
    }
}

#[test]
fn a_holder_move_needs_a_claim() {
    {
        let (from, to) = (Review, Done);
        assert_eq!(
            move_given(
                &without_prs(without_claim(task(from))),
                to,
                &person("Ada"),
                &at("10:00:00")
            ),
            Err(BoardRuleError::NoClaim),
            "{from:?} -> {to:?}"
        );
    }
}

#[test]
fn a_holder_move_on_an_expired_claim_asks_for_a_renewal_except_a_release() {
    let ada = person("Ada");
    let expired = at("11:00:00");
    for (from, to) in ALLOWED {
        let anyones = ANYONES.contains(&(from, to));
        let release = RELEASES.contains(&(from, to)) || WHILE_LIVE.contains(&(from, to));
        let moved = move_given(&without_prs(task(from)), to, &ada, &expired);
        match (anyones, release) {
            (false, false) => assert_eq!(
                moved,
                Err(BoardRuleError::ClaimExpired),
                "{from:?} -> {to:?}"
            ),
            _ => assert!(moved.is_ok(), "{from:?} -> {to:?}: {moved:?}"),
        }
    }
    let renewed = super::super::claims::renew(&without_prs(task(Review)), &ada, &expired).unwrap();
    assert!(move_given(&renewed, Done, &ada, &expired).is_ok());
}

#[test]
fn anyone_readies_a_review_or_blocked_task_whose_claim_is_absent_or_expired() {
    let bob = person("Bob");
    for from in [Review, Blocked] {
        for (held, now) in [
            (task(from), at("11:00:00")),
            (without_claim(task(from)), at("10:00:00")),
        ] {
            let readied = move_given(&held, Ready, &bob, &now).unwrap();
            assert_eq!(
                (readied.fields().status, &readied.fields().claim),
                (Ready, &None),
                "{from:?}"
            );
        }
    }
}

#[test]
fn a_two_move_steal_of_a_live_claim_is_refused() {
    let bob = person("Bob");
    for from in [Review, Blocked] {
        let live = at("10:59:59");
        assert_eq!(
            move_task(&task(from), Ready, &bob, (&[], &[]), &[], &live),
            Err(BoardRuleError::NotHolder),
            "{from:?}: Bob cannot clear Ada's live claim to claim it himself"
        );
        assert_eq!(
            super::super::claims::take_over(&task(from), &bob, &live),
            Err(BoardRuleError::ClaimLive)
        );
        let released = move_given(&task(from), Ready, &person("Ada"), &live).unwrap();
        let claimed = move_task(&released, Claimed, &bob, (&[], &[]), &[], &live).unwrap();
        assert_eq!(
            claimed.fields().claim.as_ref().map(|claim| &claim.holder),
            Some(&bob),
            "only once Ada releases it"
        );
    }
}

#[test]
fn a_held_task_is_done_once_all_its_prs_merge_whoever_merged_them() {
    let merged = |prs: &[u64]| prs.iter().copied().collect::<BTreeSet<u64>>();
    // Only from review (#2705 T4), whoever holds it.
    for held in [task(Review), without_claim(task(Review))] {
        let done = complete_merged(&held, &merged(&[2482, 2483, 9]), &[], &[], &at("10:00:00"))
            .map(|done| done.task)
            .unwrap();
        let fields = done.fields();
        assert_eq!(
            (fields.status, &fields.claim, &fields.updated),
            (Done, &None, &at("10:00:00"))
        );
    }
    let partly = complete_merged(&task(Review), &merged(&[2482]), &[], &[], &at("10:00:00"))
        .map(|done| done.task);
    assert_eq!(partly, Err(BoardRuleError::PrsUnmerged(vec![2483])));
    for from in [Draft, Ready, Claimed, InProgress, Blocked, Done, Archived] {
        let refused = complete_merged(
            &task(from),
            &merged(&[2482, 2483]),
            &[],
            &[],
            &at("10:00:00"),
        )
        .map(|done| done.task);
        let way = match lifecycle_action(from, Done) {
            Some(action) => BoardRuleError::UseLifecycle {
                from,
                to: Done,
                action,
            },
            None => BoardRuleError::NotAllowed { from, to: Done },
        };
        assert_eq!(refused, Err(way), "{from:?}");
    }
    let mut unlinked = task(Review).into_fields();
    unlinked.prs.clear();
    let unlinked = Task::new(unlinked).unwrap();
    assert_eq!(
        complete_merged(&unlinked, &merged(&[2482]), &[], &[], &at("10:00:00"))
            .map(|done| done.task),
        Err(BoardRuleError::NoPrs)
    );
}

#[test]
fn a_write_from_a_fast_clock_does_not_stop_another_clients_moves() {
    // Someone's clock ran ten years ahead and stamped the task in 2036.
    let ahead = far_future();
    let bob = person("Bob");
    let now = at("10:00:00");
    let readied = move_task(
        &last_written(task(Draft), &ahead),
        Ready,
        &bob,
        (&[], &[]),
        &[],
        &now,
    )
    .unwrap();
    let second = |n: u64| ahead.plus_seconds(n).unwrap();
    assert_eq!(
        readied.fields().updated,
        second(1),
        "stamped max(now, updated + 1 s)"
    );
    let claimed = move_task(&readied, Claimed, &bob, (&[], &[]), &[], &now).unwrap();
    let claim = claimed.fields().claim.clone().unwrap();
    assert_eq!(
        (&claim.since, &claimed.fields().updated),
        (&second(2), &second(2)),
        "the claim starts at the task's own time"
    );
    assert_eq!(claim.expires, second(2 + 2 * 60 * 60));
    let released = move_task(&claimed, Ready, &bob, (&[], &[]), &[], &now).unwrap();
    assert_eq!(released.fields().updated, second(3));
    let done = complete_merged(
        &last_written(task(Review), &ahead),
        &BTreeSet::from([2482, 2483]),
        &[],
        &[],
        &now,
    )
    .map(|done| done.task)
    .unwrap();
    assert_eq!(done.fields().updated, second(1));
}

#[test]
fn expiry_is_judged_by_the_callers_clock() {
    // Ada's claim runs 09:00 to 11:00 though the task was last written in
    // 2036: at 11:00 by Bob's clock it has expired.
    let ahead = last_written(without_prs(task(Review)), &far_future());
    let taken = super::super::claims::take_over(&ahead, &person("Bob"), &at("11:00:00")).unwrap();
    assert_eq!(
        taken.fields().updated,
        far_future().plus_seconds(1).unwrap()
    );
    assert_eq!(
        move_given(&ahead, Done, &person("Ada"), &at("11:00:00")),
        Err(BoardRuleError::ClaimExpired)
    );
}

#[test]
fn the_same_committer_under_another_name_moves_the_task() {
    let renamed = Identity {
        name: "A. Lovelace".into(),
        email: "ADA@example.com".into(),
    };
    let moved = move_given(&without_prs(task(Review)), Done, &renamed, &at("10:00:00"));
    assert_eq!(moved.map(|task| task.fields().status), Ok(Done));
}

#[test]
fn a_ready_task_at_its_text_budget_is_claimed_by_a_holder_at_its_widest() {
    let mut fields = task(Ready).into_fields();
    // The title and description at their limits in four-byte characters,
    // and references for the rest of the budget, to the byte: a holder
    // counted as text would take the task over it.
    fields.title = "\u{1d11e}".repeat(MAX_TITLE_CHARS);
    fields.description = "\u{1d11e}".repeat(MAX_DESCRIPTION_CHARS);
    let mut left = MAX_TEXT_BYTES - fields.text_bytes();
    while left > 0 {
        let head = format!("https://example.com/{}/", fields.references.len());
        let len = left.min(MAX_REFERENCE_BYTES);
        assert!(len > head.len(), "room for the last reference: {len}");
        let text = format!("{head}{}", "a".repeat(len - head.len()));
        fields.references.push(Reference::parse(&text).unwrap());
        left -= len;
    }
    assert_eq!(fields.text_bytes(), MAX_TEXT_BYTES);
    let full = Task::new(fields).expect("exactly the text budget");
    let widest = Identity {
        name: "\u{1d11e}".repeat(MAX_NAME_CHARS),
        email: format!("{}@x.io", "a".repeat(MAX_EMAIL_BYTES - 5)),
    };
    let claimed = move_task(&full, Claimed, &widest, (&[], &[]), &[], &at("10:00:00")).unwrap();
    assert_eq!(
        claimed.fields().claim.as_ref().map(|claim| &claim.holder),
        Some(&widest)
    );
}

#[test]
fn a_change_that_cannot_be_spelt_is_refused_as_invalid() {
    let mut ready = task(Ready).into_fields();
    let late = Timestamp::parse("9999-12-31T23:00:00Z").unwrap();
    ready.updated = late.clone();
    let ready = Task::new(ready).unwrap();
    let moved = move_task(&ready, Claimed, &person("Bob"), (&[], &[]), &[], &late);
    assert!(
        matches!(&moved, Err(BoardRuleError::Invalid(error)) if error.field == "claim/expires"),
        "{moved:?}"
    );
}

#[test]
fn a_task_linking_pull_requests_is_done_only_when_they_merge() {
    let review = task(Review);
    assert_eq!(
        move_task(
            &review,
            Done,
            &person("Ada"),
            (&[], &[]),
            &[],
            &at("10:00:00")
        ),
        Err(BoardRuleError::PrsLinked(vec![2482, 2483])),
        "its holder cannot say it is done"
    );
    assert_eq!(
        BoardRuleError::PrsLinked(vec![2482, 2483]).to_string(),
        "the task links pull requests [2482, 2483], so it is done when they merge: \
         complete it once they have (anyone may), or unlink one first"
    );
    let mut unlinked = review.into_fields();
    unlinked.prs.clear();
    let unlinked = Task::new(unlinked).unwrap();
    let done = move_task(
        &unlinked,
        Done,
        &person("Ada"),
        (&[], &[]),
        &[],
        &at("10:00:00"),
    )
    .unwrap();
    assert_eq!(
        done.fields().status,
        Done,
        "work without pull requests is the holder's to finish"
    );
}

/// A task linking changes is done when they merge, as one linking pull
/// requests is: its holder cannot say so, and neither completion passes
/// while the other's links have not merged.
#[test]
fn a_task_linking_changes_is_done_only_when_they_merge() {
    let review = without_prs(task(Review));
    let ada = person("Ada");
    let now = at("10:00:00");
    let mut merged_one = change(2, Some(8)).into_fields();
    merged_one.merged = Some(commit('a'));
    let mut gone = change(3, None).into_fields();
    gone.unlinked = Some((ada.clone(), at("09:00:00")));
    let changes = [
        change(1, Some(7)),
        Change::new(merged_one).unwrap(),
        Change::new(gone.clone()).unwrap(),
    ];
    assert_eq!(
        move_task(&review, Done, &ada, (&[], &changes), &[], &now),
        Err(BoardRuleError::ChangesLinked(vec![1, 2]))
    );
    let unlinked_only = [Change::new(gone).unwrap()];
    assert!(move_task(&review, Done, &ada, (&[], &unlinked_only), &[], &now).is_ok());
    let both = task(Review);
    let merged = BTreeSet::from([2482, 2483]);
    assert_eq!(
        complete_merged(&both, &merged, &changes, &[], &now).map(|done| done.task),
        Err(BoardRuleError::ChangesUnmerged(vec![1])),
        "an open change holds the pull requests' completion"
    );
    assert!(complete_merged(&both, &merged, &changes[1..], &[], &now).is_ok());
    let mut theirs = change(1, None).into_fields();
    theirs.task = Slug::parse("other").unwrap();
    let theirs = [Change::new(theirs).unwrap()];
    assert_eq!(
        complete_merged(&both, &merged, &theirs, &[], &now).map(|done| done.task),
        Err(BoardRuleError::NotThisTasks)
    );
    assert_eq!(
        move_task(&review, Done, &ada, (&[], &theirs), &[], &now),
        Err(BoardRuleError::NotThisTasks)
    );
}

/// A task done is resolved as completed, whichever way it is done; an
/// archived one keeps its done one's resolution, or has none.
#[test]
fn a_done_task_is_resolved_as_completed() {
    let now = at("10:00:00");
    let done = move_task(
        &without_prs(task(Review)),
        Done,
        &person("Ada"),
        (&[], &[]),
        &[],
        &now,
    )
    .unwrap();
    assert_eq!(done.fields().resolution, Some(Resolution::Completed));
    let merged = BTreeSet::from([2482, 2483]);
    let completed = complete_merged(&task(Review), &merged, &[], &[], &now).unwrap();
    assert_eq!(
        completed.task.fields().resolution,
        Some(Resolution::Completed)
    );
    let archived = move_task(&done, Archived, &person("Bob"), (&[], &[]), &[], &now).unwrap();
    assert_eq!(archived.fields().resolution, Some(Resolution::Completed));
    let ready = without_prs(task(Ready));
    let dropped = move_task(&ready, Archived, &person("Bob"), (&[], &[]), &[], &now).unwrap();
    assert_eq!(dropped.fields().resolution, None);
}

/// A run still going keeps its task held: leaving for ready, done or
/// archived waits until it ends.
#[test]
fn a_task_with_a_run_going_stays_with_its_holder() {
    let running = crate::domain::project_board::services::fixtures_tests::open_run(1, 1);
    let ada = person("Ada");
    for (from, to) in RELEASES.iter().copied() {
        let reason = match from {
            Review => MoveReason::Given("Back for a new plan".into()),
            _ => MoveReason::Absent,
        };
        assert_eq!(
            move_with_reason(
                &without_prs(task(from)),
                (to, &reason),
                &ada,
                (std::slice::from_ref(&running), &[]),
                &[],
                &at("10:00:00")
            ),
            Err(BoardRuleError::RunOpen(1)),
            "{from:?} -> {to:?}"
        );
    }
}

/// A merged pull request closes the task whoever holds it: a run still
/// going can no longer report its end to a done task, so it is ended as
/// lost at the change's stamp, and none is left open.
#[test]
fn completing_a_task_ends_its_open_run_as_lost() {
    let merged = BTreeSet::from([2482, 2483]);
    let running = crate::domain::project_board::services::fixtures_tests::open_run(1, 1);
    {
        let from = Review;
        let done = complete_merged(
            &task(from),
            &merged,
            &[],
            std::slice::from_ref(&running),
            &at("10:00:00"),
        )
        .unwrap();
        let run = done.run.expect("the run going is ended");
        assert_eq!(
            (
                done.task.fields().status,
                &run.fields().ended,
                run.fields().outcome,
                &done.task.fields().claim
            ),
            (
                Done,
                &Some(at("10:00:00")),
                Some(crate::domain::project_board::value_objects::vocabulary::RunOutcome::Lost),
                &None
            ),
            "{from:?}"
        );
    }
    let quiet = complete_merged(&task(Review), &merged, &[], &[], &at("10:00:00")).unwrap();
    assert_eq!(quiet.run, None);
}

#[test]
fn a_completion_refuses_another_tasks_run_and_two_going() {
    let merged = BTreeSet::from([2482, 2483]);
    let mut elsewhere =
        crate::domain::project_board::services::fixtures_tests::open_run(1, 1).into_fields();
    elsewhere.task =
        crate::domain::project_board::value_objects::slug::Slug::parse("other").unwrap();
    let elsewhere = crate::domain::project_board::entities::run::Run::new(elsewhere).unwrap();
    assert_eq!(
        complete_merged(
            &task(Review),
            &merged,
            &[],
            std::slice::from_ref(&elsewhere),
            &at("10:00:00")
        )
        .map(|done| done.task),
        Err(BoardRuleError::NotThisTasks)
    );
    let both = [
        crate::domain::project_board::services::fixtures_tests::open_run(1, 1),
        crate::domain::project_board::services::fixtures_tests::open_run(2, 1),
    ];
    assert!(matches!(
        complete_merged(&task(Review), &merged, &[], &both, &at("10:00:00")),
        Err(BoardRuleError::RunsInconsistent(_))
    ));
}

/// A move reads of the task's work only what can hold it: the runs for a
/// move to a status that holds no claim, the changes for review to done,
/// nothing among the held statuses; a move the rule refuses anyway is
/// refused before anything is read.
#[test]
fn a_move_reads_only_what_can_hold_it() {
    let ada = person("Ada");
    let now = at("10:00:00");
    let reads = |from, to| move_reads(&without_prs(task(from)), to, &ada, &now);
    let read = |runs, changes| Ok(MoveReads { runs, changes });
    assert_eq!(reads(Ready, Claimed), read(false, false));
    assert_eq!(reads(Claimed, Ready), read(true, false));
    assert_eq!(reads(Review, Done), read(true, true));
    assert_eq!(reads(Review, Archived), read(true, true));
    assert_eq!(
        reads(Ready, Review),
        Err(BoardRuleError::NotAllowed {
            from: Ready,
            to: Review
        })
    );
    assert_eq!(
        reads(Claimed, InProgress),
        Err(BoardRuleError::UseLifecycle {
            from: Claimed,
            to: InProgress,
            action: crate::domain::project_board::value_objects::vocabulary::LifecycleAction::Start
        })
    );
    assert_eq!(
        move_reads(&task(Review), Done, &ada, &now),
        Err(BoardRuleError::PrsLinked(task(Review).fields().prs.clone())),
        "a task linking pull requests is refused before its changes are read"
    );
    assert_eq!(
        move_reads(&without_prs(task(Review)), Done, &person("Bob"), &now),
        Err(BoardRuleError::NotHolder)
    );
    assert_eq!(
        completable(&task(Draft)),
        Err(BoardRuleError::NotAllowed {
            from: Draft,
            to: Done
        })
    );
}

/// For every move the table allows, a run going refuses it exactly when
/// [`move_reads`] says the move reads the runs: what is read and what
/// holds the task are one rule.
#[test]
fn a_run_going_holds_exactly_the_moves_that_read_runs() {
    let ada = person("Ada");
    let now = at("10:00:00");
    let running = crate::domain::project_board::services::fixtures_tests::open_run(1, 1);
    for (from, to, ..) in TRANSITIONS {
        let held = without_prs(task(*from));
        let reads = move_reads(&held, *to, &ada, &now).unwrap();
        let reason = match (from, to) {
            (Review, Ready) => MoveReason::Given("Back for a new plan".into()),
            _ => MoveReason::Absent,
        };
        let moved = move_with_reason(
            &held,
            (*to, &reason),
            &ada,
            (std::slice::from_ref(&running), &[]),
            &[],
            &now,
        );
        assert_eq!(
            moved == Err(BoardRuleError::RunOpen(1)),
            reads.runs,
            "{from:?} -> {to:?}: {moved:?}"
        );
        assert_eq!(reads.runs, super::super::runs::holds_moves_to(*to));
    }
}
