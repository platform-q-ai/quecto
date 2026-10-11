use super::*;
use crate::domain::project_board::entities::claim::{MAX_EMAIL_BYTES, MAX_NAME_CHARS};
use crate::domain::project_board::entities::task::{MAX_DESCRIPTION_CHARS, MAX_TEXT_BYTES};
use crate::domain::project_board::entities::task_parts::{MAX_LINE_CHARS, PlanStep};
use crate::domain::project_board::services::fixtures_tests::{
    at, far_future, last_written, person, task, with_open_run, without_claim, without_prs,
};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::vocabulary::RunOutcome;

const ALL: [TaskStatus; 8] = [
    Draft, Ready, Claimed, InProgress, Review, Blocked, Done, Archived,
];
/// Written out independently of the production table.
const ALLOWED: [(TaskStatus, TaskStatus); 17] = [
    (Draft, Ready),
    (Draft, Archived),
    (Ready, Draft),
    (Ready, Claimed),
    (Ready, Archived),
    (Claimed, InProgress),
    (Claimed, Blocked),
    (Claimed, Ready),
    (InProgress, Review),
    (InProgress, Blocked),
    (InProgress, Ready),
    (Review, Done),
    (Review, InProgress),
    (Review, Ready),
    (Blocked, InProgress),
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
/// The moves that hand a task back to the board.
const RELEASES: [(TaskStatus, TaskStatus); 4] = [
    (Claimed, Ready),
    (InProgress, Ready),
    (Review, Ready),
    (Blocked, Ready),
];

#[test]
fn only_the_table_allows_a_move_even_for_the_holder() {
    for from in ALL {
        for to in ALL {
            let moved = move_task(
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
                    assert_eq!(error, BoardRuleError::NotAllowed { from, to });
                }
            }
        }
    }
}

#[test]
fn anyone_plans_readies_claims_and_archives_but_only_the_holder_moves_a_live_claim() {
    let bob = person("Bob");
    for (from, to) in ALLOWED {
        let moved = move_task(&task(from), to, &bob, &at("10:00:00"));
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
    assert!(move_task(&task(Claimed), InProgress, &shouting, &at("10:00:00")).is_ok());
}

#[test]
fn a_claim_is_taken_kept_or_cleared_by_the_move() {
    let now = at("10:00:00");
    let claimed = move_task(&task(Ready), Claimed, &person("Bob"), &now).unwrap();
    let claim = claimed.fields().claim.clone().unwrap();
    assert_eq!(
        (claim.holder, claim.since, claim.expires),
        (person("Bob"), now.clone(), at("12:00:00"))
    );
    for (from, to) in [
        (Claimed, InProgress),
        (Claimed, Blocked),
        (InProgress, Blocked),
        (Blocked, InProgress),
        (Review, InProgress),
    ] {
        let moved = move_task(&task(from), to, &person("Ada"), &now).unwrap();
        assert_eq!(
            moved.fields().claim,
            task(from).fields().claim,
            "{from:?} -> {to:?} keeps the claim"
        );
    }
    let cleared = RELEASES.iter().chain(&[(Review, Done)]);
    for (from, to) in cleared.copied() {
        let next = move_task(&without_prs(task(from)), to, &person("Ada"), &now).unwrap();
        assert_eq!(next.fields().claim, None, "{from:?} -> {to:?} clears it");
    }
}

#[test]
fn a_holder_move_needs_a_claim() {
    for (from, to) in [(Review, Done), (Review, InProgress), (Blocked, InProgress)] {
        assert_eq!(
            move_task(
                &without_claim(task(from)),
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
        let release = RELEASES.contains(&(from, to));
        let moved = move_task(&task(from), to, &ada, &expired);
        match (anyones, release) {
            (false, false) => assert_eq!(
                moved,
                Err(BoardRuleError::ClaimExpired),
                "{from:?} -> {to:?}"
            ),
            _ => assert!(moved.is_ok(), "{from:?} -> {to:?}: {moved:?}"),
        }
    }
    let renewed = super::super::claims::renew(&task(Claimed), &ada, &expired).unwrap();
    assert!(move_task(&renewed, InProgress, &ada, &expired).is_ok());
}

#[test]
fn anyone_readies_a_review_or_blocked_task_whose_claim_is_absent_or_expired() {
    let bob = person("Bob");
    for from in [Review, Blocked] {
        for (held, now) in [
            (task(from), at("11:00:00")),
            (without_claim(task(from)), at("10:00:00")),
        ] {
            let readied = move_task(&held, Ready, &bob, &now).unwrap();
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
            move_task(&task(from), Ready, &bob, &live),
            Err(BoardRuleError::NotHolder),
            "{from:?}: Bob cannot clear Ada's live claim to claim it himself"
        );
        assert_eq!(
            super::super::claims::take_over(&task(from), &bob, &live),
            Err(BoardRuleError::ClaimLive)
        );
        let released = move_task(&task(from), Ready, &person("Ada"), &live).unwrap();
        let claimed = move_task(&released, Claimed, &bob, &live).unwrap();
        assert_eq!(
            claimed.fields().claim.as_ref().map(|claim| &claim.holder),
            Some(&bob),
            "only once Ada releases it"
        );
    }
}

#[test]
fn an_open_run_stays_through_moves_that_keep_the_task_held() {
    let ada = person("Ada");
    for (from, to) in [
        (Claimed, InProgress),
        (Claimed, Blocked),
        (InProgress, Review),
        (InProgress, Blocked),
        (Review, InProgress),
        (Blocked, InProgress),
    ] {
        let moved = move_task(&with_open_run(task(from)), to, &ada, &at("10:00:00")).unwrap();
        assert!(
            moved.fields().runs.iter().any(|run| run.is_open()),
            "{from:?} -> {to:?}"
        );
    }
}

#[test]
fn a_move_to_ready_done_or_archived_with_an_open_run_is_refused() {
    let open = BoardRuleError::RunOpen(Slug::parse("r2").unwrap());
    for (from, to) in RELEASES.iter().chain(&[(Review, Done)]).copied() {
        assert_eq!(
            move_task(
                &with_open_run(without_prs(task(from))),
                to,
                &person("Ada"),
                &at("10:00:00")
            ),
            Err(open.clone()),
            "{from:?} -> {to:?}"
        );
    }
}

/// A merged pull request closes the task whoever holds it: a run still
/// open can no longer report its end to a done task, so it is ended as
/// lost, at the change's stamp, as a run that will never report is.
#[test]
fn completing_a_task_ends_its_open_run_as_lost() {
    let merged = BTreeSet::from([2482, 2483]);
    for from in [Claimed, InProgress, Review, Blocked] {
        let done = complete_merged(&with_open_run(task(from)), &merged, &at("10:00:00")).unwrap();
        let run = done
            .fields()
            .runs
            .iter()
            .find(|run| run.id.as_str() == "r2")
            .unwrap();
        assert_eq!(
            (
                done.fields().status,
                &run.ended,
                run.outcome,
                &done.fields().claim
            ),
            (Done, &Some(at("10:00:00")), Some(RunOutcome::Lost), &None),
            "{from:?}"
        );
        assert_eq!(
            done.fields().runs[0].outcome,
            Some(RunOutcome::Succeeded),
            "an ended run keeps its outcome"
        );
    }
}

#[test]
fn a_held_task_is_done_once_all_its_prs_merge_whoever_merged_them() {
    let merged = |prs: &[u64]| prs.iter().copied().collect::<BTreeSet<u64>>();
    // Ready too: a holder released it with its pull requests linked, and
    // no one holds it when they merge.
    for from in [Ready, Claimed, InProgress, Review, Blocked] {
        let done =
            complete_merged(&task(from), &merged(&[2482, 2483, 9]), &at("10:00:00")).unwrap();
        let fields = done.fields();
        assert_eq!(
            (fields.status, &fields.claim, &fields.updated),
            (Done, &None, &at("10:00:00")),
            "{from:?}"
        );
    }
    let partly = complete_merged(&task(Review), &merged(&[2482]), &at("10:00:00"));
    assert_eq!(partly, Err(BoardRuleError::PrsUnmerged(vec![2483])));
    for from in [Draft, Done, Archived] {
        let refused = complete_merged(&task(from), &merged(&[2482, 2483]), &at("10:00:00"));
        assert_eq!(refused, Err(BoardRuleError::NotAllowed { from, to: Done }));
    }
    let mut unlinked = task(Review).into_fields();
    unlinked.prs.clear();
    let unlinked = Task::new(unlinked).unwrap();
    assert_eq!(
        complete_merged(&unlinked, &merged(&[2482]), &at("10:00:00")),
        Err(BoardRuleError::NoPrs)
    );
}

#[test]
fn a_write_from_a_fast_clock_does_not_stop_another_clients_moves() {
    // Someone's clock ran ten years ahead and stamped the task in 2036.
    let ahead = far_future();
    let bob = person("Bob");
    let now = at("10:00:00");
    let readied = move_task(&last_written(task(Draft), &ahead), Ready, &bob, &now).unwrap();
    let second = |n: u64| ahead.plus_seconds(n).unwrap();
    assert_eq!(
        readied.fields().updated,
        second(1),
        "stamped max(now, updated + 1 s)"
    );
    let claimed = move_task(&readied, Claimed, &bob, &now).unwrap();
    let claim = claimed.fields().claim.clone().unwrap();
    assert_eq!(
        (&claim.since, &claimed.fields().updated),
        (&second(2), &second(2)),
        "the claim starts at the task's own time"
    );
    assert_eq!(claim.expires, second(2 + 2 * 60 * 60));
    let started = move_task(&claimed, InProgress, &bob, &now).unwrap();
    assert_eq!(started.fields().status, InProgress);
    let done = complete_merged(
        &last_written(task(Review), &ahead),
        &BTreeSet::from([2482, 2483]),
        &now,
    )
    .unwrap();
    assert_eq!(done.fields().updated, second(1));
}

#[test]
fn expiry_is_judged_by_the_callers_clock() {
    // Ada's claim runs 09:00 to 11:00 though the task was last written in
    // 2036: at 11:00 by Bob's clock it has expired.
    let ahead = last_written(task(InProgress), &far_future());
    let taken = super::super::claims::take_over(&ahead, &person("Bob"), &at("11:00:00")).unwrap();
    assert_eq!(
        taken.fields().updated,
        far_future().plus_seconds(1).unwrap()
    );
    assert_eq!(
        move_task(&ahead, Review, &person("Ada"), &at("11:00:00")),
        Err(BoardRuleError::ClaimExpired)
    );
}

#[test]
fn the_same_committer_under_another_name_moves_the_task_and_keeps_the_claim() {
    let renamed = Identity {
        name: "A. Lovelace".into(),
        email: "ADA@example.com".into(),
    };
    let moved = move_task(&task(Claimed), InProgress, &renamed, &at("10:00:00")).unwrap();
    assert_eq!(
        moved.fields().claim,
        task(Claimed).fields().claim,
        "the holder keeps the name it claimed under"
    );
}

#[test]
fn a_ready_task_at_its_text_budget_is_claimed_by_a_holder_at_its_widest() {
    let mut fields = task(Ready).into_fields();
    let mut left = MAX_TEXT_BYTES - fields.text_bytes();
    let wide = (left / 4).min(MAX_DESCRIPTION_CHARS);
    fields.description = "\u{1d11e}".repeat(wide);
    left -= 4 * wide;
    while left >= 4 {
        let chars = (left / 4).min(MAX_LINE_CHARS);
        fields.plan.push(PlanStep {
            title: "\u{1d11e}".repeat(chars),
            criteria: vec![],
        });
        left -= 4 * chars;
    }
    fields.plan.push(PlanStep {
        title: "x".repeat(left.max(1)),
        criteria: vec![],
    });
    let full = Task::new(fields).expect("at most the text budget");
    let widest = Identity {
        name: "\u{1d11e}".repeat(MAX_NAME_CHARS),
        email: format!("{}@x.io", "a".repeat(MAX_EMAIL_BYTES - 5)),
    };
    let claimed = move_task(&full, Claimed, &widest, &at("10:00:00")).unwrap();
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
    let moved = move_task(&ready, Claimed, &person("Bob"), &late);
    assert!(
        matches!(&moved, Err(BoardRuleError::Invalid(error)) if error.field == "claim/expires"),
        "{moved:?}"
    );
}

#[test]
fn a_task_linking_pull_requests_is_done_only_when_they_merge() {
    let review = task(Review);
    assert_eq!(
        move_task(&review, Done, &person("Ada"), &at("10:00:00")),
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
    let done = move_task(&unlinked, Done, &person("Ada"), &at("10:00:00")).unwrap();
    assert_eq!(
        done.fields().status,
        Done,
        "work without pull requests is the holder's to finish"
    );
}
