use super::*;

/// #1855: usage the harness could not price reads as unpriced, never $0.
#[test]
fn unpriced_usage_reads_as_unpriced() {
    let text =
        |data: serde_json::Value| session_stats_text(&session_payloads::parse_session_stats(&data));
    let unknown = text(serde_json::json!({"costMicroUsd": null, "unpricedAttempts": 1}));
    assert!(
        unknown.contains("Cost: unpriced") && !unknown.contains('$'),
        "{unknown}"
    );
    let part = text(serde_json::json!({"costMicroUsd": 5, "unpricedAttempts": 1}));
    assert!(part.contains("Cost: $0.000005 + unpriced"), "{part}");
    let free = text(serde_json::json!({"costMicroUsd": 0, "unpricedAttempts": 0}));
    assert!(!free.contains("Cost"), "{free}");
}

/// #2635 review: a known $0 (a declared-free model) beside unpriced usage
/// shows as "$0.000000 + unpriced", never as wholly unpriced.
#[test]
fn a_known_zero_cost_beside_unpriced_usage_is_shown() {
    let stats = session_payloads::parse_session_stats(
        &serde_json::json!({"costMicroUsd": 0, "unpricedAttempts": 1}),
    );
    let text = session_stats_text(&stats);
    assert!(text.contains("Cost: $0.000000 + unpriced"), "{text}");
}
