//! #2705 T4: `move` makes only the moves people make; the lifecycle makes
//! the rest (`start`, a run's end, `done_without_run`), and `move` refuses
//! each naming the action to use. Back to ready from review takes a
//! reason, but for the CLI's unasked one (the Pocket relay's moves).
use super::*;
use crate::domain::project_board::entities::claim::Claim;
use crate::domain::project_board::services::fixtures_tests::{
    at, change, open_run, person, task, without_claim, without_prs,
};
use crate::domain::project_board::value_objects::vocabulary::LifecycleAction;

const ALL: [TaskStatus; 8] = [
    Draft, Ready, Claimed, InProgress, Review, Blocked, Done, Archived,
];

/// Every move `move` makes, written out independently of the table.
const ALLOWED: [(TaskStatus, TaskStatus); 12] = [
    (Draft, Ready),
    (Draft, Archived),
    (Ready, Draft),
    (Ready, Claimed),
    (Ready, Archived),
    (Claimed, Ready),
    (InProgress, Ready),
    (Review, Done),
    (Review, Ready),
    (Review, Archived),
    (Blocked, Ready),
    (Done, Archived),
];

/// Every move the lifecycle makes, and the action `move` names for it.
const LIFECYCLE: [(TaskStatus, TaskStatus, LifecycleAction); 9] = [
    (Ready, InProgress, LifecycleAction::Start),
    (Claimed, InProgress, LifecycleAction::Start),
    (Review, InProgress, LifecycleAction::Start),
    (Blocked, InProgress, LifecycleAction::Start),
    (InProgress, Review, LifecycleAction::EndRun),
    (InProgress, Blocked, LifecycleAction::EndRun),
    (Claimed, Blocked, LifecycleAction::Release),
    (Ready, Done, LifecycleAction::DoneWithoutRun),
    (Claimed, Done, LifecycleAction::DoneWithoutRun),
];

fn reason(text: &str) -> MoveReason {
    MoveReason::Given(text.into())
}

/// The reason each move needs: back to ready from review, one.
fn reason_for(from: TaskStatus, to: TaskStatus) -> MoveReason {
    match (from, to) {
        (Review, Ready) => reason("The pull request was closed: the approach changed"),
        _ => MoveReason::Absent,
    }
}

fn moved(task: &Task, to: TaskStatus, why: &MoveReason, actor: &str, time: &str) -> MoveResult {
    move_with_reason(task, to, why, &person(actor), (&[], &[]), &[], &at(time))
}

type MoveResult = Result<Task, BoardRuleError>;

#[test]
fn every_pair_is_made_by_move_by_the_lifecycle_or_by_nothing() {
    for from in ALL {
        for to in ALL {
            let held = without_prs(task(from));
            let answer = moved(&held, to, &reason_for(from, to), "Ada", "10:00:00");
            let lifecycle = LIFECYCLE
                .iter()
                .find(|(row_from, row_to, _)| (*row_from, *row_to) == (from, to));
            match (ALLOWED.contains(&(from, to)), lifecycle) {
                (true, None) => {
                    let next = answer.unwrap_or_else(|e| panic!("{from:?} -> {to:?}: {e}"));
                    assert_eq!(next.fields().status, to, "{from:?} -> {to:?}");
                }
                (false, Some((_, _, action))) => assert_eq!(
                    answer,
                    Err(BoardRuleError::UseLifecycle {
                        from,
                        to,
                        action: *action
                    }),
                    "{from:?} -> {to:?}"
                ),
                (false, None) => assert_eq!(
                    answer,
                    Err(BoardRuleError::NotAllowed { from, to }),
                    "{from:?} -> {to:?}"
                ),
                (true, Some(_)) => panic!("{from:?} -> {to:?} is listed twice"),
            }
        }
    }
}

#[test]
fn a_move_to_in_progress_names_start_whoever_asks() {
    for from in [Ready, Claimed, Review, Blocked] {
        for actor in ["Ada", "Bob"] {
            assert_eq!(
                moved(
                    &task(from),
                    InProgress,
                    &MoveReason::Absent,
                    actor,
                    "10:00:00"
                ),
                Err(BoardRuleError::UseLifecycle {
                    from,
                    to: InProgress,
                    action: LifecycleAction::Start
                }),
                "{from:?} by {actor}"
            );
        }
    }
}

#[test]
fn a_move_to_review_names_end_run() {
    assert_eq!(
        moved(
            &task(InProgress),
            Review,
            &MoveReason::Absent,
            "Ada",
            "10:00:00"
        ),
        Err(BoardRuleError::UseLifecycle {
            from: InProgress,
            to: Review,
            action: LifecycleAction::EndRun
        })
    );
    let refused = moved(
        &task(InProgress),
        Review,
        &MoveReason::Absent,
        "Ada",
        "10:00:00",
    );
    let message = refused.unwrap_err().to_string();
    assert!(message.contains("end_run(task, run, outcome)"), "{message}");
}

#[test]
fn back_to_ready_from_review_needs_a_reason_and_keeps_it() {
    let review = without_prs(task(Review));
    assert_eq!(
        moved(&review, Ready, &MoveReason::Absent, "Ada", "10:00:00"),
        Err(BoardRuleError::ReasonRequired {
            from: Review,
            to: Ready
        })
    );
    let why = "The pull request was closed: the approach changed";
    let back = moved(&review, Ready, &reason(why), "Ada", "10:00:00").unwrap();
    let fields = back.fields();
    assert_eq!(
        (fields.status, fields.reason.as_deref(), &fields.claim),
        (Ready, Some(why), &None)
    );
    // An empty reason says nothing.
    let empty = moved(&review, Ready, &reason(""), "Ada", "10:00:00");
    match empty {
        Err(BoardRuleError::Invalid(error)) => assert_eq!(error.field, "reason"),
        other => panic!("an empty reason is refused: {other:?}"),
    }
    // A reason never normalised is refused, as every entered line is.
    let spaced = moved(&review, Ready, &reason(" padded "), "Ada", "10:00:00");
    match spaced {
        Err(BoardRuleError::Invalid(error)) => assert_eq!(error.field, "reason"),
        other => panic!("a reason never normalised is refused: {other:?}"),
    }
}

#[test]
fn a_reason_is_taken_only_by_a_move_back_to_ready_from_review() {
    for (from, to) in [
        (Claimed, Ready),
        (Draft, Ready),
        (Ready, Claimed),
        (Done, Archived),
    ] {
        let held = without_prs(task(from));
        match moved(&held, to, &reason("why"), "Ada", "10:00:00") {
            Err(BoardRuleError::Invalid(error)) => {
                assert_eq!(error.field, "reason", "{from:?} -> {to:?}");
            }
            other => panic!("{from:?} -> {to:?} takes no reason: {other:?}"),
        }
    }
}

#[test]
fn every_other_move_clears_the_reason() {
    let back = moved(
        &without_prs(task(Review)),
        Ready,
        &reason("Back for a new plan"),
        "Ada",
        "10:00:00",
    )
    .unwrap();
    for to in [Draft, Claimed, Archived] {
        let next = moved(&back, to, &MoveReason::Absent, "Bob", "10:30:00").unwrap();
        assert_eq!(next.fields().reason, None, "ready -> {to:?}");
    }
}

#[test]
fn done_without_run_finishes_a_ready_or_claimed_task_with_its_reason() {
    let why = "Answered in chat: the flag already exists";
    let ready = without_prs(task(Ready));
    let done = done_without_run(&ready, why, &person("Bob"), (&[], &[]), &at("10:00:00")).unwrap();
    let fields = done.fields();
    assert_eq!(
        (
            fields.status,
            fields.resolution,
            fields.reason.as_deref(),
            &fields.claim,
            &fields.updated
        ),
        (
            Done,
            Some(Resolution::DoneWithoutRun),
            Some(why),
            &None,
            &at("10:00:00")
        )
    );
    let claimed = without_prs(task(Claimed));
    let done = done_without_run(&claimed, why, &person("Ada"), (&[], &[]), &at("10:00:00"));
    assert_eq!(done.map(|task| task.fields().status), Ok(Done));
    // Only the holder finishes a claimed task, while the claim is live.
    assert_eq!(
        done_without_run(&claimed, why, &person("Bob"), (&[], &[]), &at("10:00:00")),
        Err(BoardRuleError::NotHolder)
    );
    assert_eq!(
        done_without_run(&claimed, why, &person("Ada"), (&[], &[]), &at("14:00:00")),
        Err(BoardRuleError::ClaimExpired)
    );
}

#[test]
fn done_without_run_without_a_reason_is_refused() {
    for why in ["", " padded ", "two\nlines"] {
        let refused = done_without_run(
            &without_prs(task(Ready)),
            why,
            &person("Ada"),
            (&[], &[]),
            &at("10:00:00"),
        );
        match refused {
            Err(BoardRuleError::Invalid(error)) => assert_eq!(error.field, "reason", "{why:?}"),
            other => panic!("{why:?} is no reason: {other:?}"),
        }
    }
}

#[test]
fn done_without_run_is_only_for_work_no_run_or_link_finishes() {
    let why = "Done by hand";
    let ada = person("Ada");
    let now = at("10:00:00");
    for from in [Draft, InProgress, Review, Blocked, Done, Archived] {
        assert_eq!(
            done_without_run(&without_prs(task(from)), why, &ada, (&[], &[]), &now),
            Err(BoardRuleError::NotAllowed { from, to: Done }),
            "{from:?}"
        );
    }
    // A task with pull requests or changes linked is done when they merge.
    assert_eq!(
        done_without_run(&task(Claimed), why, &ada, (&[], &[]), &now),
        Err(BoardRuleError::PrsLinked(vec![2482, 2483]))
    );
    let linked = [change(1, Some(9001))];
    assert_eq!(
        done_without_run(&without_prs(task(Claimed)), why, &ada, (&[], &linked), &now),
        Err(BoardRuleError::ChangesLinked(vec![1]))
    );
    // A run going holds the task.
    let going = [open_run(1, 1)];
    assert_eq!(
        done_without_run(&without_prs(task(Claimed)), why, &ada, (&going, &[]), &now),
        Err(BoardRuleError::RunOpen(1))
    );
}

/// The relay's bot holds what its runs start.
fn held_by_bot(status: TaskStatus) -> Task {
    let mut fields = without_prs(task(status)).into_fields();
    fields.claim = Some(Claim {
        holder: person("Bot"),
        since: at("09:00:00"),
        expires: at("11:00:00"),
    });
    Task::new(fields).unwrap()
}

/// Every move the live Pocket relay sends `move` stays accepted, as it
/// sends it: through the CLI, so with no reason (`Unasked`).
#[test]
fn every_move_the_pocket_relay_sends_stays_accepted() {
    let unasked = MoveReason::Unasked;
    // Mark ready, Back to draft, and accept-and-start's move to ready.
    for (from, to) in [(Draft, Ready), (Ready, Draft)] {
        let next = moved(&without_prs(task(from)), to, &unasked, "Bot", "10:00:00");
        assert_eq!(
            next.map(|t| t.fields().status),
            Ok(to),
            "{from:?} -> {to:?}"
        );
    }
    // Back to ready from review: the bot's, nobody's, or another's lapsed.
    let review = [
        (held_by_bot(Review), "10:00:00"),
        (without_claim(without_prs(task(Review))), "10:00:00"),
        (without_prs(task(Review)), "14:00:00"),
    ];
    for (held, time) in review {
        let next = moved(&held, Ready, &unasked, "Bot", time).unwrap();
        assert_eq!(
            (next.fields().status, &next.fields().reason),
            (Ready, &None)
        );
    }
    // A Stop's release: claimed or in progress, held by the bot, its run
    // ended (none going).
    for from in [Claimed, InProgress] {
        let next = moved(&held_by_bot(from), Ready, &unasked, "Bot", "10:00:00");
        assert_eq!(next.map(|t| t.fields().status), Ok(Ready), "{from:?}");
    }
    // Mark done of a review the bot holds, linking nothing.
    let done = moved(&held_by_bot(Review), Done, &unasked, "Bot", "10:00:00");
    assert_eq!(done.map(|t| t.fields().status), Ok(Done));
    // Delete task: an archive of a draft, ready, review or done task.
    for (held, from) in [
        (without_prs(task(Draft)), Draft),
        (without_prs(task(Ready)), Ready),
        (held_by_bot(Review), Review),
        (without_prs(task(Done)), Done),
    ] {
        let next = moved(&held, Archived, &unasked, "Bot", "10:00:00");
        assert_eq!(next.map(|t| t.fields().status), Ok(Archived), "{from:?}");
    }
}

/// A Stop's release still waits for the run to end.
#[test]
fn a_release_from_progress_waits_for_its_run() {
    let going = [open_run(1, 1)];
    assert_eq!(
        move_with_reason(
            &held_by_bot(InProgress),
            Ready,
            &MoveReason::Unasked,
            &person("Bot"),
            (&going, &[]),
            &[],
            &at("10:00:00"),
        ),
        Err(BoardRuleError::RunOpen(1))
    );
}

#[test]
fn a_merged_pull_request_completes_a_task_only_from_review() {
    assert_eq!(completable(&task(Review)), Ok(()));
    for from in [Draft, Ready, Claimed, InProgress, Blocked, Done, Archived] {
        assert_eq!(
            completable(&task(from)),
            Err(BoardRuleError::NotAllowed { from, to: Done }),
            "{from:?}"
        );
    }
}
