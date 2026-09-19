use super::*;
use crate::domain::session_home::SessionHomeScope;

fn decision(kind: ResumeDecisionKind, capabilities: &ResumeActionCapabilities) -> ResumeDecision {
    ResumeDecision {
        target: ResumeTarget::parse("cli:elsewhere").unwrap(),
        kind,
        home_version: HomeVersion::of(
            &ResumeTarget::parse("cli:elsewhere").unwrap().identity,
            &SessionHomeScope::LegacyUnscoped,
        ),
        execution_dir: None,
        detail: None,
        offers: kind
            .offered_actions()
            .iter()
            .map(|action| ResumeActionOffer {
                action: *action,
                availability: capabilities.availability(*action),
            })
            .collect(),
    }
}

#[test]
fn a_restore_request_is_the_exact_key_with_no_action_or_version() {
    let request = ResumeRequest::restore("chat-1-a");
    assert_eq!(request.target, "chat-1-a");
    assert_eq!(request.intent, ResumeIntent::Restore);
    assert_eq!(request.expected_home_version, None);
}

#[test]
fn only_cancel_is_executable_until_an_executor_is_declared() {
    let none = ResumeActionCapabilities::cancel_only();
    for action in ResumeAction::ALL {
        let available = none.availability(action) == ActionAvailability::Available;
        assert_eq!(available, action == ResumeAction::Cancel, "{action:?}");
    }
    let forks = none.clone().with(ResumeAction::ForkCurrent);
    assert_eq!(
        forks.availability(ResumeAction::ForkCurrent),
        ActionAvailability::Available
    );
    assert!(matches!(
        forks.availability(ResumeAction::OpenOriginal),
        ActionAvailability::Unavailable(_)
    ));
    assert_ne!(none, forks);
}

#[test]
fn every_unavailable_reason_says_what_to_do_is_distinct_and_names_no_issue() {
    let none = ResumeActionCapabilities::cancel_only();
    let reasons: Vec<String> = [
        (ResumeAction::OpenOriginal, "start quecto in that folder"),
        (ResumeAction::ForkCurrent, "conversation stays as it is"),
        (ResumeAction::Locate, "The saved conversation is kept"),
        (ResumeAction::Associate, "stays listed under All Folders"),
    ]
    .into_iter()
    .map(|(action, slice)| match none.availability(action) {
        ActionAvailability::Unavailable(reason) => {
            assert!(reason.contains(slice), "{reason}");
            // The fact first, and no internal vocabulary (review R2-T7).
            assert!(reason.starts_with("Not available yet. "), "{reason}");
            for internal in ["runtime", "legacy", "association", "transcript"] {
                assert!(!reason.contains(internal), "{internal:?} in {reason}");
            }
            // For people: no internal tracker number reaches the dialog.
            assert!(!reason.contains('#'), "{reason}");
            reason
        }
        ActionAvailability::Available => panic!("{action:?} is not executable"),
    })
    .collect();
    let distinct: std::collections::BTreeSet<_> = reasons.iter().collect();
    assert_eq!(distinct.len(), 4);
    assert_eq!(
        unavailable_reason(ResumeAction::Cancel),
        "cancel is always available"
    );
}

#[test]
fn a_decision_names_the_obstacle_every_offer_and_the_first_reason() {
    let none = ResumeActionCapabilities::cancel_only();
    let text = decision(ResumeDecisionKind::CrossFolder, &none).to_string();
    assert_eq!(
        text.split(". ").next().unwrap(),
        "session resume unavailable: session belongs to a different execution directory; \
         choose open_original (unavailable) / fork_current (unavailable) / cancel"
    );
    assert!(text.contains("start quecto in that folder"), "{text}");
    assert!(!text.contains('#'), "{text}");
    let legacy = decision(ResumeDecisionKind::LegacyUnscoped, &none).to_string();
    assert!(
        legacy.contains("associate (unavailable) / cancel"),
        "{legacy}"
    );
    assert!(
        legacy.contains("it stays listed under All Folders"),
        "{legacy}"
    );
}

#[test]
fn a_decision_whose_offers_are_all_executable_gives_no_reason() {
    let all = ResumeActionCapabilities::cancel_only()
        .with(ResumeAction::OpenOriginal)
        .with(ResumeAction::ForkCurrent);
    let decision = decision(ResumeDecisionKind::CrossFolder, &all);
    assert_eq!(
        decision.to_string(),
        "session resume unavailable: session belongs to a different execution directory; \
         choose open_original / fork_current / cancel"
    );
    assert!(decision.offer(ResumeAction::Cancel).is_some());
    assert!(decision.offer(ResumeAction::Locate).is_none());
}
