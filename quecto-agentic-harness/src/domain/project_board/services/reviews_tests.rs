use super::*;
use crate::domain::project_board::entities::change::ChangeFields;
use crate::domain::project_board::entities::review::MAX_ROUNDS;
use crate::domain::project_board::services::fixtures_tests::{at, branch, change, commit, person};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::Severity;

fn when(time: &str) -> Moment {
    Moment {
        now: at(time),
        latest: None,
    }
}

fn draft(verdict: ReviewVerdict) -> RoundDraft {
    RoundDraft {
        commit: commit('a'),
        verdict,
        findings: vec![Finding {
            location: Some("src/store.rs:42".into()),
            severity: Severity::High,
            text: "Lease not checked".into(),
        }],
    }
}

fn first(target: &Change) -> Review {
    add_round(
        target,
        None,
        draft(ReviewVerdict::ChangesRequested),
        &person("Bob"),
        &when("10:00:00"),
    )
    .unwrap()
}

/// A change's first round begins its review; later rounds follow, each
/// stamped after the last, and the state is the last round's verdict.
#[test]
fn rounds_build_one_review_per_change_in_order() {
    let target = change(1, Some(7));
    let begun = first(&target);
    let fields = begun.fields();
    assert_eq!(
        (
            &fields.task,
            fields.change,
            &fields.created,
            &fields.updated,
            begun.state()
        ),
        (
            &target.fields().task,
            1,
            &at("10:00:00"),
            &at("10:00:00"),
            ReviewVerdict::ChangesRequested
        )
    );
    let mut later = draft(ReviewVerdict::Approved);
    later.commit = commit('b');
    later.findings.clear();
    let second = add_round(
        &target,
        Some(&begun),
        later,
        &person("Carol"),
        &when("10:00:00"),
    )
    .unwrap();
    let fields = second.fields();
    assert_eq!(second.state(), ReviewVerdict::Approved);
    assert_eq!(
        fields.rounds[0],
        begun.fields().rounds[0],
        "earlier rounds kept"
    );
    let round = &fields.rounds[1];
    assert_eq!(
        (
            &round.commit,
            &round.reviewer,
            &round.at,
            round.findings.len()
        ),
        (&commit('b'), &person("Carol"), &at("10:00:01"), 0),
        "after the round before"
    );
    assert_eq!(
        (&fields.created, &fields.updated),
        (&at("10:00:00"), &at("10:00:01"))
    );
    let third = add_round(
        &target,
        Some(&second),
        draft(ReviewVerdict::Approved),
        &person("Bob"),
        &when("11:00:00"),
    )
    .unwrap();
    assert_eq!(
        third.fields().updated,
        at("11:00:00"),
        "a clock ahead is kept"
    );
}

/// A merged change stays linked, and is still reviewed.
#[test]
fn a_merged_change_is_still_reviewed() {
    let mut merged = change(1, Some(7)).into_fields();
    merged.merged = Some(commit('c'));
    assert!(
        add_round(
            &Change::new(merged).unwrap(),
            None,
            draft(ReviewVerdict::Approved),
            &person("Bob"),
            &when("10:00:00")
        )
        .is_ok()
    );
}

#[test]
fn a_round_is_refused_on_an_unlinked_change_or_another_changes_review() {
    let target = change(1, Some(7));
    let review = first(&target);
    let mut gone = target.clone().into_fields();
    gone.unlinked = Some((person("Ada"), at("09:00:00")));
    let gone = Change::new(gone).unwrap();
    assert_eq!(
        add_round(
            &gone,
            None,
            draft(ReviewVerdict::Approved),
            &person("Bob"),
            &when("10:00:00")
        ),
        Err(BoardRuleError::ChangeUnlinked(1))
    );
    let numbered = Change::new(ChangeFields {
        number: 2,
        branch: branch("feat/two"),
        ..target.clone().into_fields()
    })
    .unwrap();
    let tasked = Change::new(ChangeFields {
        task: Slug::parse("other").unwrap(),
        ..target.into_fields()
    })
    .unwrap();
    for other in [numbered, tasked] {
        assert_eq!(
            add_round(
                &other,
                Some(&review),
                draft(ReviewVerdict::Approved),
                &person("Bob"),
                &when("10:00:00")
            ),
            Err(BoardRuleError::NotThisChange)
        );
    }
}

/// A review a clock running ahead stamped is refused once an outside
/// time bounds the clock, and taken within the stamp's slack.
#[test]
fn a_review_stamped_ahead_is_refused() {
    let target = change(1, Some(7));
    let begun = add_round(
        &target,
        None,
        draft(ReviewVerdict::ChangesRequested),
        &person("Bob"),
        &when("10:30:00"),
    )
    .unwrap();
    let adding = |latest: &str| {
        let bounded = Moment {
            now: at("10:00:00"),
            latest: Some(at(latest)),
        };
        add_round(
            &target,
            Some(&begun),
            draft(ReviewVerdict::Approved),
            &person("Bob"),
            &bounded,
        )
    };
    assert_eq!(
        adding("10:00:00"),
        Err(BoardRuleError::ReviewAhead {
            change: 1,
            updated: at("10:30:00")
        })
    );
    assert_eq!(
        adding("10:29:29"),
        Err(BoardRuleError::ReviewAhead {
            change: 1,
            updated: at("10:30:00")
        }),
        "a second past the slack"
    );
    let taken = adding("10:29:30").unwrap();
    assert_eq!(
        taken.fields().updated,
        at("10:30:01"),
        "after the round before"
    );
}

#[test]
fn a_round_its_schema_refuses_is_refused() {
    let mut bad = draft(ReviewVerdict::Approved);
    bad.findings[0].text = " ".into();
    let refused = add_round(
        &change(1, Some(7)),
        None,
        bad,
        &person("Bob"),
        &when("10:00:00"),
    );
    assert!(
        matches!(&refused, Err(BoardRuleError::Invalid(error)) if error.field == "rounds/0/findings/0/text"),
        "{refused:?}"
    );
}

/// A review never predates its change: a first round from a clock behind
/// is stamped when the change was linked.
#[test]
fn a_first_round_is_stamped_no_earlier_than_its_change() {
    let target = change(1, Some(7));
    let early = add_round(
        &target,
        None,
        draft(ReviewVerdict::Approved),
        &person("Bob"),
        &when("08:00:00"),
    )
    .unwrap();
    let fields = early.fields();
    assert_eq!(
        (&fields.created, &fields.updated, &fields.rounds[0].at),
        (&at("09:00:00"), &at("09:00:00"), &at("09:00:00"))
    );
}

/// A review last changed at the last moment the board can spell takes no
/// later round: the overflow is refused, never wrapped.
#[test]
fn a_review_at_the_end_of_time_takes_no_round() {
    let target = change(1, Some(7));
    let end = Timestamp::parse("9999-12-31T23:59:59Z").unwrap();
    let mut fields = first(&target).into_fields();
    fields.created = end.clone();
    fields.updated = end.clone();
    fields.rounds[0].at = end;
    let review = Review::new(fields).unwrap();
    let refused = add_round(
        &target,
        Some(&review),
        draft(ReviewVerdict::Approved),
        &person("Bob"),
        &when("10:00:00"),
    );
    assert!(
        matches!(&refused, Err(BoardRuleError::Invalid(error)) if error.field == "updated"),
        "{refused:?}"
    );
}

/// The verdict is the reviewer's call: an approval may carry a critical
/// finding, a round may follow an approval, and two rounds may judge one
/// commit.
#[test]
fn the_verdict_is_the_reviewers_call() {
    let target = change(1, Some(7));
    let mut critical = draft(ReviewVerdict::Approved);
    critical.findings[0].severity = Severity::Critical;
    let approved = add_round(&target, None, critical, &person("Bob"), &when("10:00:00")).unwrap();
    assert_eq!(approved.state(), ReviewVerdict::Approved);
    let again = add_round(
        &target,
        Some(&approved),
        draft(ReviewVerdict::ChangesRequested),
        &person("Carol"),
        &when("10:00:00"),
    )
    .unwrap();
    let rounds = &again.fields().rounds;
    assert_eq!((rounds.len(), &rounds[0].commit), (2, &rounds[1].commit));
    assert_eq!(again.state(), ReviewVerdict::ChangesRequested);
}

/// A first round is stamped no earlier than its change, so a change a
/// clock running ahead stamped is refused once an outside time bounds the
/// clock, and taken within the slack.
#[test]
fn a_first_round_on_a_change_stamped_ahead_is_refused() {
    let mut fields = change(1, Some(7)).into_fields();
    fields.created = at("10:30:00");
    fields.updated = at("10:30:00");
    let fast = Change::new(fields).unwrap();
    let first_round = |latest: &str| {
        let bounded = Moment {
            now: at("10:00:00"),
            latest: Some(at(latest)),
        };
        add_round(
            &fast,
            None,
            draft(ReviewVerdict::Approved),
            &person("Bob"),
            &bounded,
        )
    };
    assert_eq!(
        first_round("10:29:29"),
        Err(BoardRuleError::ChangeAhead {
            change: 1,
            updated: at("10:30:00")
        })
    );
    let taken = first_round("10:29:30").unwrap();
    assert_eq!(taken.fields().created, at("10:30:00"));
}

/// A review holds at most its most rounds: the next is refused.
#[test]
fn a_review_takes_no_round_past_its_most() {
    let target = change(1, Some(7));
    let mut review = first(&target);
    for _ in 1..MAX_ROUNDS {
        review = add_round(
            &target,
            Some(&review),
            draft(ReviewVerdict::Approved),
            &person("Bob"),
            &when("10:00:00"),
        )
        .unwrap();
    }
    assert_eq!(review.fields().rounds.len(), MAX_ROUNDS);
    let refused = add_round(
        &target,
        Some(&review),
        draft(ReviewVerdict::Approved),
        &person("Bob"),
        &when("10:00:00"),
    );
    assert!(
        matches!(&refused, Err(BoardRuleError::Invalid(error)) if error.field == "rounds"),
        "{refused:?}"
    );
}
