use super::*;
use crate::domain::project_board::entities::run::Run;
use crate::domain::project_board::services::fixtures_tests::{
    at, change, commit, open_run, person, task, without_claim, without_prs,
};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::vocabulary::RunOutcome;

/// Ada's claim runs 09:00 to 11:00: live at 10:00, lapsed hours ago at
/// 14:00.
fn live() -> Timestamp {
    at("10:00:00")
}

fn lapsed() -> Timestamp {
    at("14:00:00")
}

/// A task in review whose run has ended, and which links no pull request
/// and no change, is archived by its holder while the claim is live, and
/// by anyone once it has lapsed or is gone (as review to ready): the claim
/// is cleared and the task is unresolved, as a cancelled one is. While
/// the claim is live, nobody else archives it.
#[test]
fn a_reviewed_task_is_archived_by_its_live_holder_or_by_anyone_once_nobody_holds_it() {
    let mut ended = open_run(1, 1).into_fields();
    ended.ended = Some(at("09:30:00"));
    ended.outcome = Some(RunOutcome::Succeeded);
    let ended = Run::new(ended).unwrap();
    let reviewed = without_prs(task(Review));
    let unclaimed = without_claim(reviewed.clone());
    for (held, actor, now) in [
        (&reviewed, person("Ada"), live()),
        (&reviewed, person("Bob"), lapsed()),
        (&reviewed, person("Ada"), lapsed()),
        (&unclaimed, person("Bob"), live()),
    ] {
        let archived = move_task(
            held,
            Archived,
            &actor,
            (std::slice::from_ref(&ended), &[]),
            &[],
            &now,
        )
        .unwrap();
        let fields = archived.fields();
        assert_eq!(
            (fields.status, &fields.claim, fields.resolution),
            (Archived, &None, None),
            "{actor:?} at {now:?}"
        );
        assert!(fields.prs.is_empty(), "it pins no PR");
    }
    let bob = person("Bob");
    assert_eq!(
        move_reads(&reviewed, Archived, &bob, &live()),
        Err(BoardRuleError::NotHolder),
        "refused before anything is read"
    );
    assert_eq!(
        move_task(
            &reviewed,
            Archived,
            &bob,
            (std::slice::from_ref(&ended), &[]),
            &[],
            &live()
        ),
        Err(BoardRuleError::NotHolder),
        "Bob, while Ada's claim is live"
    );
}

/// A run still going holds a reviewed task back for everyone who may
/// archive it, its live holder included, so an archive never races a live
/// run.
#[test]
fn a_run_going_holds_a_reviewed_task_back_from_archive() {
    let running = open_run(1, 1);
    let reviewed = without_prs(task(Review));
    let unclaimed = without_claim(reviewed.clone());
    for (held, actor, now) in [
        (&reviewed, person("Ada"), live()),
        (&reviewed, person("Bob"), lapsed()),
        (&unclaimed, person("Bob"), live()),
    ] {
        assert_eq!(
            move_task(
                held,
                Archived,
                &actor,
                (std::slice::from_ref(&running), &[]),
                &[],
                &now
            ),
            Err(BoardRuleError::RunOpen(1)),
            "{actor:?} at {now:?}"
        );
    }
}

/// A reviewed task that links pull requests is not archived, so an
/// archived task pins none that has not merged: refused as review to done is, before
/// its runs or changes are read.
#[test]
fn a_reviewed_task_linking_pull_requests_is_not_archived() {
    let linked = BoardRuleError::PrsLinked(vec![2482, 2483]);
    for (actor, now) in [(person("Ada"), live()), (person("Bob"), lapsed())] {
        assert_eq!(
            move_reads(&task(Review), Archived, &actor, &now),
            Err(linked.clone()),
            "{actor:?}: refused before anything is read"
        );
        assert_eq!(
            move_task(&task(Review), Archived, &actor, (&[], &[]), &[], &now),
            Err(linked.clone()),
            "{actor:?}"
        );
    }
}

/// A reviewed task a change links (open or merged, as review to done
/// counts them) is not archived; one whose changes are all unlinked is.
/// The archive reads the task's changes as review to done does.
#[test]
fn a_reviewed_task_a_change_links_is_not_archived() {
    let reviewed = without_prs(task(Review));
    assert_eq!(
        move_reads(&reviewed, Archived, &person("Ada"), &live()),
        Ok(MoveReads {
            runs: true,
            changes: true
        })
    );
    let mut merged = change(2, Some(8)).into_fields();
    merged.merged = Some(commit('a'));
    let mut gone = change(3, None).into_fields();
    gone.unlinked = Some((person("Ada"), at("09:00:00")));
    let gone = Change::new(gone).unwrap();
    let changes = [
        change(1, Some(7)),
        Change::new(merged).unwrap(),
        gone.clone(),
    ];
    for (actor, now) in [(person("Ada"), live()), (person("Bob"), lapsed())] {
        assert_eq!(
            move_task(&reviewed, Archived, &actor, (&[], &changes), &[], &now),
            Err(BoardRuleError::ChangesLinked(vec![1, 2])),
            "{actor:?}"
        );
    }
    let archived = move_task(
        &reviewed,
        Archived,
        &person("Bob"),
        (&[], std::slice::from_ref(&gone)),
        &[],
        &lapsed(),
    )
    .unwrap();
    assert_eq!(archived.fields().status, Archived);
}

/// Releasing a reviewed task first (review to ready, and on to draft)
/// never gets its links archived: a ready or draft task is archived on
/// review's terms, whoever made the steps (Ada while her claim is live,
/// Bob once it has lapsed). Unlinked, it is archived.
#[test]
fn a_released_task_is_not_archived_while_it_links_pull_requests_or_changes() {
    let changes = [change(1, Some(7))];
    for (actor, now) in [(person("Ada"), live()), (person("Bob"), lapsed())] {
        let back = MoveReason::Given("Back for a new plan".into());
        let ready =
            move_with_reason(&task(Review), (Ready, &back), &actor, (&[], &[]), &[], &now).unwrap();
        assert_eq!(
            (ready.fields().status, &ready.fields().prs),
            (Ready, &vec![2482, 2483]),
            "{actor:?}: the release keeps the links"
        );
        let draft = move_task(&ready, Draft, &actor, (&[], &[]), &[], &now).unwrap();
        for released in [&ready, &draft] {
            let from = released.fields().status;
            assert_eq!(
                move_reads(released, Archived, &actor, &now),
                Err(BoardRuleError::PrsLinked(vec![2482, 2483])),
                "{actor:?} {from:?}: refused before anything is read"
            );
            assert_eq!(
                move_task(released, Archived, &actor, (&[], &[]), &[], &now),
                Err(BoardRuleError::PrsLinked(vec![2482, 2483])),
                "{actor:?} {from:?}"
            );
        }
        let unlinked = move_with_reason(
            &without_prs(task(Review)),
            (Ready, &back),
            &actor,
            (&[], &[]),
            &[],
            &now,
        )
        .unwrap();
        assert_eq!(
            move_reads(&unlinked, Archived, &actor, &now),
            Ok(MoveReads {
                runs: true,
                changes: true
            })
        );
        assert_eq!(
            move_task(&unlinked, Archived, &actor, (&[], &changes), &[], &now),
            Err(BoardRuleError::ChangesLinked(vec![1])),
            "{actor:?}: a change linked holds it too"
        );
        let archived = move_task(&unlinked, Archived, &actor, (&[], &[]), &[], &now).unwrap();
        assert_eq!(archived.fields().status, Archived, "{actor:?}");
    }
}

/// A task others took over is archived as superseded, never done: its
/// holder's move and the completion by merged pull requests are refused,
/// naming who took it over.
#[test]
fn a_superseded_task_is_never_done() {
    let superseded = |task: Task| {
        let mut fields = task.into_fields();
        fields.superseded_by = vec![Slug::parse("next").unwrap()];
        Task::new(fields).unwrap()
    };
    let refused = Err(BoardRuleError::Superseded {
        by: vec![Slug::parse("next").unwrap()],
    });
    let now = at("10:00:00");
    let review = superseded(without_prs(task(Review)));
    assert_eq!(
        move_task(&review, Done, &person("Ada"), (&[], &[]), &[], &now),
        refused
    );
    let merged = BTreeSet::from([2482, 2483]);
    let linked = superseded(task(Review));
    assert_eq!(
        complete_merged(&linked, &merged, &[], &[], &now).map(|done| done.task),
        refused
    );
    let back = MoveReason::Given("Back for a new plan".into());
    assert!(
        move_with_reason(
            &review,
            (Ready, &back),
            &person("Ada"),
            (&[], &[]),
            &[],
            &now
        )
        .is_ok(),
        "it is released, as any reviewed task"
    );
    let linking = superseded(task(Review));
    assert_eq!(
        move_task(&linking, Done, &person("Ada"), (&[], &[]), &[], &now),
        refused,
        "superseded is said before the pull requests it links"
    );
}

/// A task others took over is archived as superseded, as the refusal of
/// its completion advises; one archived otherwise is unresolved.
#[test]
fn a_superseded_task_is_archived_as_superseded() {
    let now = at("10:00:00");
    for status in [Ready, Draft, Review] {
        let mut fields = without_prs(task(status)).into_fields();
        fields.superseded_by = vec![Slug::parse("next").unwrap()];
        let archived = move_task(
            &Task::new(fields).unwrap(),
            Archived,
            &person("Ada"),
            (&[], &[]),
            &[],
            &now,
        )
        .unwrap();
        assert_eq!(
            archived.fields().resolution,
            Some(Resolution::Superseded),
            "{status:?}"
        );
        let plain = move_task(
            &without_prs(task(status)),
            Archived,
            &person("Ada"),
            (&[], &[]),
            &[],
            &now,
        )
        .unwrap();
        assert_eq!(plain.fields().resolution, None, "{status:?}");
    }
}
