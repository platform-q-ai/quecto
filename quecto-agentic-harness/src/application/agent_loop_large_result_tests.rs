//! #2348: the size-aware collapse on the scripted coordinator of #2342
//! (one ~17k-token bash output at turn 1, a growing full summary every
//! 6th turn, ~420-token results otherwise). Measured as estimated message
//! tokens per request, with #2342's superseded snapshots and 48k swarm
//! ceiling on in both runs: the saving is the rule's alone.

use super::snapshot_tests::{AFTER, LARGE_HEAD, SECRET, Setup, run_scripted};
use crate::application::context_pruning::large_results::LargeResultCollapse;
use crate::domain::audit::AuditEvent;

/// The shipped default: over 2k estimated tokens, seen for 3 turns.
const SHIPPED: LargeResultCollapse = LargeResultCollapse {
    over_tokens: 2_000,
    after_turns: 3,
};

fn fewer(before: usize, after: usize) -> f64 {
    100.0 * (1.0 - after as f64 / before as f64)
}

#[tokio::test]
async fn a_large_result_collapses_after_three_turns_at_the_shipped_dial() {
    let before = run_scripted(AFTER).await;
    let after = run_scripted(Setup {
        large: SHIPPED,
        ..AFTER
    })
    .await;

    let (before_tokens, after_tokens) =
        (before.total_request_tokens(), after.total_request_tokens());
    let requests = after.provider.large_in_full.lock().unwrap().clone();
    eprintln!(
        "#2348 scripted coordinator, tool dial 50 ({} requests): before {before_tokens}, after {after_tokens} ({:.1}% fewer)",
        requests.len(),
        fewer(before_tokens, after_tokens)
    );
    for dial in [100, 25] {
        let base = run_scripted(Setup { dial, ..AFTER }).await;
        let with = run_scripted(Setup {
            dial,
            large: SHIPPED,
            ..AFTER
        })
        .await;
        let (b, a) = (base.total_request_tokens(), with.total_request_tokens());
        eprintln!(
            "#2348   tool dial {dial}: {b} -> {a} ({:.1}% fewer)",
            fewer(b, a)
        );
    }
    assert_eq!(
        before.provider.large_in_full.lock().unwrap().len(),
        requests.len(),
        "the same conversation: the model's calls do not change"
    );
    assert!(
        after_tokens * 10 < before_tokens * 8,
        "at least 20% fewer at the shipped tool dial of 50: before {before_tokens}, after {after_tokens}"
    );
    // Request 1 precedes the output; requests 2-4 carry it in full (the
    // model sees it three times); from request 5 on it is a recall stub.
    assert_eq!(&requests[..6], &[false, true, true, true, false, false]);
    assert!(requests[4..].iter().all(|&full| !full), "{requests:?}");
    assert!(
        after
            .provider
            .latest_summary_visible
            .lock()
            .unwrap()
            .iter()
            .all(|&visible| visible),
        "the newest summary stays in full on every request"
    );
}

#[tokio::test]
async fn the_prune_record_counts_large_results_and_carries_no_content() {
    let after = run_scripted(Setup {
        large: SHIPPED,
        ..AFTER
    })
    .await;

    let events = after.sink.events.lock().unwrap();
    let large: Vec<usize> = events
        .iter()
        .filter_map(|event| match event {
            AuditEvent::ContextPruned {
                large_results_collapsed,
                ..
            } => Some(*large_results_collapsed),
            _ => None,
        })
        .collect();
    // Only the bash output is over 2k: every older summary was superseded
    // first, and the newest is never collapsed.
    assert_eq!(large.iter().sum::<usize>(), 1, "records: {large:?}");
    for event in events.iter() {
        let json = serde_json::to_string(event).unwrap();
        if json.contains("context_pruned") {
            assert!(json.contains("\"large_results_collapsed\""), "{json}");
            assert!(!json.contains(SECRET), "a prune record carries counts only");
            assert!(!json.contains(LARGE_HEAD), "no result content: {json}");
        }
    }
}

#[tokio::test]
async fn with_the_rule_off_no_large_result_is_collapsed() {
    let before = run_scripted(AFTER).await;

    let events = before.sink.events.lock().unwrap();
    assert!(events.iter().all(|event| !matches!(
        event,
        AuditEvent::ContextPruned {
            large_results_collapsed: 1..,
            ..
        }
    )));
    let requests = before.provider.large_in_full.lock().unwrap();
    assert!(
        requests[1..10].iter().all(|&full| full),
        "without the rule the output rides every early request"
    );
}
