//! #2708: the turn-outcome wire shapes.
use super::*;
use serde_json::json;

fn record() -> TurnOutcomeRecord {
    TurnOutcomeRecord {
        session_key: String::new(),
        outcome_epoch: "epoch".into(),
        episode_id: "episode".into(),
        turn_id: "turn".into(),
        sequence: 3,
        outcome: TurnOutcome::NeedsUser,
        reason: "Which repository?".into(),
    }
}

/// Review #2762 (8) and (B): the operator-approved wording, verbatim.
#[test]
fn issue_2708_parent_runtime_the_approved_wording_is_exact() {
    assert_eq!(
        CORRECTION_NOTE,
        "Nothing is running. Launch the next step, or call declare_turn_outcome to say you're blocked or done."
    );
    assert_eq!(
        DECLARE_GUIDANCE,
        "End every turn by calling declare_turn_outcome: done when you've answered or finished, asking when you need the user's answer, blocked when you can't proceed, waiting only while work you launched is running."
    );
}

#[test]
fn issue_2708_output_projection_event_is_exactly_eight_keys() {
    let event = serde_json::to_value(TurnOutcomeEvent::TurnOutcome(record())).unwrap();
    assert_eq!(
        event,
        json!({"type":"turn_outcome","sessionKey":"","outcomeEpoch":"epoch","episodeId":"episode",
            "turnId":"turn","sequence":3,"outcome":"needs_user","reason":"Which repository?"})
    );
    let back: TurnOutcomeEvent = serde_json::from_value(event).unwrap();
    assert_eq!(back, TurnOutcomeEvent::TurnOutcome(record()));
}

#[test]
fn issue_2708_output_projection_outcomes_are_a_closed_allowlist() {
    for (outcome, wire) in [
        (TurnOutcome::Waiting, "waiting"),
        (TurnOutcome::NeedsUser, "needs_user"),
        (TurnOutcome::Stalled, "stalled"),
        (TurnOutcome::Done, "done"),
    ] {
        assert_eq!(serde_json::to_value(outcome).unwrap(), json!(wire));
    }
    let mut alien = serde_json::to_value(TurnOutcomeEvent::TurnOutcome(record())).unwrap();
    alien["outcome"] = json!("alien");
    assert!(serde_json::from_value::<TurnOutcomeEvent>(alien).is_err());
    let mut extra = serde_json::to_value(TurnOutcomeEvent::TurnOutcome(record())).unwrap();
    extra["extra"] = json!(1);
    assert!(serde_json::from_value::<TurnOutcomeEvent>(extra).is_err());
}
