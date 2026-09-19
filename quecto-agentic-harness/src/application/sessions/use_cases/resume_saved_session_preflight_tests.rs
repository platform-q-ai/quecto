//! The effect-free pre-flight and the admission of explicit actions (#2011
//! review R1-H4, R1-H10): an absent target is answered before any effect, no
//! transcript is read to decide, and an action must name the version it acts on.
use super::*;
use std::sync::atomic::Ordering;

/// The scope would be a decision — the branch the pre-flight's existence
/// check guards: an absent target is `not_found`, never a decision about a
/// record that does not exist, and nothing was settled, saved or claimed.
#[tokio::test]
async fn an_absent_target_is_not_found_before_any_effect_whatever_its_scope_says() {
    for scope in [
        SessionHomeScope::Scoped(folder(ELSEWHERE)),
        SessionHomeScope::LegacyUnscoped,
        SessionHomeScope::Scoped(folder(HERE)),
    ] {
        let case = case(scope);
        let refused = case
            .request(&ResumeRequest::restore("absent"))
            .await
            .expect_err("not found");
        assert!(
            matches!(&refused, ResumeSavedSessionError::NotFound(name) if name == "absent"),
            "{refused:?}"
        );
        assert_eq!(*case.facts.reads.lock().unwrap(), 0, "nothing to decide");
        case.assert_no_effect();
    }
}

/// A decision about a present target never loads its transcript.
#[tokio::test]
async fn a_decision_reads_no_transcript() {
    let case = case(SessionHomeScope::Scoped(folder(ELSEWHERE)));
    decision_of(case.restore().await);
    let journal = case.rig.journal();
    assert!(
        !journal.iter().any(|entry| entry.starts_with("store.load(")),
        "{journal:?}"
    );
}

/// The loop's own key may have no record yet — the departing save is what
/// first writes it — so its absence is not decided early: the transaction
/// runs and the claimed path answers.
#[tokio::test]
async fn the_loops_own_absent_key_is_left_to_the_claimed_path() {
    let case = case_on(
        "cli:unsaved",
        vec![SessionHomeScope::LegacyUnscoped],
        false,
        false,
    );
    let refused = case
        .request(&ResumeRequest::restore("unsaved"))
        .await
        .expect_err("the fake store writes no record");
    assert!(matches!(refused, ResumeSavedSessionError::NotFound(_)));
    let journal = case.rig.journal();
    assert!(
        journal
            .iter()
            .any(|entry| entry.starts_with("store.load(cli:unsaved")),
        "the claimed path ran: {journal:?}"
    );
    let released = case.rig.store.released.lock().unwrap().clone();
    assert!(
        released.is_empty(),
        "the loop's own claim is kept: {released:?}"
    );
}

/// The target is there for the pre-flight and gone under the claim (a peer
/// deleted it): the claimed path answers, and releases the claim just taken.
#[tokio::test]
async fn a_target_that_vanishes_before_the_claim_is_not_found_and_released() {
    let case = case(SessionHomeScope::Scoped(folder(HERE)));
    case.rig.store.vanish_on_claim.store(true, Ordering::SeqCst);
    let refused = case.restore().await.expect_err("not found");
    assert!(matches!(&refused, ResumeSavedSessionError::NotFound(name) if name == "saved"));
    let journal = case.rig.journal();
    assert_eq!(
        journal[journal.len() - 3..],
        [
            "store.claim(cli:saved)",
            "store.load(cli:saved)",
            "store.release(cli:saved)@active=cli:departing",
        ],
        "{journal:?}"
    );
}

/// A store that cannot answer decides nothing early; the claimed load reports.
#[tokio::test]
async fn an_unreadable_store_is_reported_by_the_claimed_path() {
    let case = case(SessionHomeScope::Scoped(folder(ELSEWHERE)));
    case.rig.store.fail_load.store(true, Ordering::SeqCst);
    let refused = case.restore().await.expect_err("load failed");
    assert!(
        matches!(refused, ResumeSavedSessionError::Load(_)),
        "{refused:?}"
    );
    assert_eq!(
        case.rig.journal().last().unwrap(),
        "store.release(cli:saved)@active=cli:departing"
    );
}

/// An explicit action acts on the authority the client was shown: without
/// that version it is refused before anything — for a composed executor too —
/// and is never replaced by a restore. Cancel needs none.
#[tokio::test]
async fn an_action_without_the_version_it_was_shown_is_refused() {
    for action in ResumeAction::ALL {
        let case = case(SessionHomeScope::Scoped(folder(HERE)));
        let request = ResumeRequest {
            target: "saved".into(),
            intent: ResumeIntent::Act(action),
            expected_home_version: None,
        };
        let result = case.request(&request).await;
        if action == ResumeAction::Cancel {
            assert!(matches!(result, Ok(ResumeOutcome::Cancelled { .. })));
        } else {
            let refused = result.expect_err("version required");
            assert!(
                matches!(&refused, ResumeSavedSessionError::HomeVersionRequired(a) if *a == action),
                "{action:?}: {refused:?}"
            );
            assert_eq!(refused.code(), "home_version_required");
            assert!(refused.to_string().contains(action.name()), "{refused}");
        }
        assert_eq!(*case.facts.reads.lock().unwrap(), 0);
        case.assert_no_effect();
    }
}
