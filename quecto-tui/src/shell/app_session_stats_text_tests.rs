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
