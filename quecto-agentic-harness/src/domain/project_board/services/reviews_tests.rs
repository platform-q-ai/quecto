use super::*;
use crate::domain::project_board::entities::change::ChangeFields;
use crate::domain::project_board::services::fixtures_tests::{at, branch, change, commit, person};
use crate::domain::project_board::value_objects::slug::Slug;
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
            Err(BoardRuleError::NotThisTasks)
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
