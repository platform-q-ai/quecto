//! Pure policy tests (#1221 acceptance criterion 1).
//!
//! The turn-recovery trigger and the batch atomicity invariant, exercised with
//! no terminal, concrete client, raw JSON, or Tokio runtime.

use super::*;

fn refs(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("ref-{i}")).collect()
}

fn outcome<'a>(refs: &'a [String], text: &'a str, tools: usize) -> TurnOutcome<'a> {
    TurnOutcome {
        refs,
        assistant_text: text,
        tools_this_turn: tools,
        open_tool_calls: 0,
        expected_content_len: None,
    }
}

#[test]
fn empty_refs_can_never_trigger_recovery() {
    let none: Vec<String> = Vec::new();
    let mut o = outcome(&none, "", 0);
    o.open_tool_calls = 3;
    o.expected_content_len = Some(9_000);
    assert!(
        !o.needs_recovery(),
        "there is nothing to rebuild a turn from without refs"
    );
}

#[test]
fn an_empty_or_ellipsis_body_triggers_recovery() {
    let r = refs(1);
    for text in ["", "   ", "…", "..."] {
        assert!(
            outcome(&r, text, 0).needs_recovery(),
            "a placeholder body ({text:?}) must be rebuilt"
        );
    }
}

#[test]
fn a_body_shorter_than_advertised_triggers_recovery() {
    let r = refs(1);
    let mut o = outcome(&r, "short", 0);
    o.expected_content_len = Some(9_000);
    assert!(o.needs_recovery());
}

#[test]
fn a_body_meeting_the_advertised_length_does_not_trigger_recovery() {
    let r = refs(1);
    let text = "a complete assistant body";
    let mut o = outcome(&r, text, 0);
    o.expected_content_len = Some(text.len() as u64);
    assert!(
        !o.needs_recovery(),
        "meeting the advertised length exactly must satisfy the check"
    );
}

#[test]
fn matching_ref_cardinality_does_not_trigger_recovery() {
    // Each tool contributes a call and a result, plus the final assistant message.
    for tools in 0..4 {
        let r = refs(tools * 2 + 1);
        assert!(
            !outcome(&r, "a complete body", tools).needs_recovery(),
            "{tools} tools with {} refs is a complete turn",
            r.len()
        );
    }
}

#[test]
fn ref_cardinality_off_by_one_in_either_direction_triggers_recovery() {
    for tools in 0..4 {
        let expected = tools * 2 + 1;
        // count 0 is excluded: empty refs are governed by the earlier rule
        // (nothing to rebuild from), not by cardinality.
        for count in [expected - 1, expected + 1].into_iter().filter(|c| *c > 0) {
            let r = refs(count);
            assert!(
                outcome(&r, "a complete body", tools).needs_recovery(),
                "{tools} tools with {count} refs (expected {expected}) lost messages"
            );
        }
    }
}

#[test]
fn an_open_tool_call_forces_recovery_despite_otherwise_complete_evidence() {
    let r = refs(1);
    let text = "a complete body";
    let mut o = outcome(&r, text, 0);
    o.expected_content_len = Some(text.len() as u64);
    assert!(
        !o.needs_recovery(),
        "control: this turn looks complete on every other signal"
    );

    o.open_tool_calls = 1;
    assert!(
        o.needs_recovery(),
        "an unmatched tool call means the stream was cut mid-turn, so the \
         rendered text cannot be trusted however plausible it looks"
    );
}

#[test]
fn a_batch_is_incomplete_until_every_ref_responds() {
    let mut batch = RecoveryBatch::new(refs(3), 4, 7, None);
    assert!(!batch.is_complete());

    batch.responses.insert("ref-0".into(), "a");
    batch.responses.insert("ref-1".into(), "b");
    assert!(
        !batch.is_complete(),
        "a partially answered batch must never replace the turn range"
    );

    batch.responses.insert("ref-2".into(), "c");
    assert!(batch.is_complete());
}

#[test]
fn responses_are_read_back_in_ref_order_not_arrival_order() {
    let mut batch = RecoveryBatch::new(refs(3), 0, 3, None);
    batch.responses.insert("ref-2".into(), "third");
    batch.responses.insert("ref-0".into(), "first");
    batch.responses.insert("ref-1".into(), "second");

    assert_eq!(
        batch.ordered_responses().copied().collect::<Vec<_>>(),
        ["first", "second", "third"],
        "the turn must be rebuilt in stream order, not response-arrival order"
    );
}
