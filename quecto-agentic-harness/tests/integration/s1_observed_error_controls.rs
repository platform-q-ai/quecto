//! Explicit error controls supplement the rate/billing raw boundary matrix.
use super::*;
use serde_json::json;

#[tokio::test]
async fn s1_observed_error_budget_unknown_and_contradiction() {
    for (leaf, surface, admitted) in cells() {
        for contradictory in [false, true] {
            for bytes in [4096, 4097] {
                let error = if contradictory {
                    json!({"type":"rate_limit_error","code":"insufficient_quota","message":""})
                } else {
                    json!({"type":"CANARY_UNKNOWN","code":"CANARY_UNKNOWN","message":""})
                };
                let mut value = match leaf {
                    Leaf::OpenAi | Leaf::Anthropic => json!({"type":"error","error":error}),
                    _ => json!({"type":"response.failed","response":{"error":error}}),
                };
                let padding = bytes - value.to_string().len() - 1;
                let message = format!("CANARY{}", "x".repeat(padding - 6));
                match leaf {
                    Leaf::OpenAi | Leaf::Anthropic => value["error"]["message"] = message.into(),
                    _ => value["response"]["error"]["message"] = message.into(),
                }
                let raw = format!("{value}\t");
                assert_eq!(raw.len(), bytes);
                let body = format!(
                    "{}data: {raw}\n\n",
                    if matches!(leaf, Leaf::Anthropic) {
                        "event: error\n"
                    } else {
                        ""
                    }
                );
                let (output, trace, state) = observe(leaf, surface, admitted, body).await;
                let error = outward(&output);
                let allowed = bytes == 4096;
                assert_eq!(
                    error.class,
                    if allowed && contradictory {
                        ProviderErrorClass::Billing
                    } else {
                        ProviderErrorClass::Server
                    }
                );
                assert_eq!(
                    error.status,
                    Some(if allowed && contradictory { 402 } else { 502 })
                );
                assert!(!error.display.contains("CANARY"));
                let attempts = trace.attempt_diagnostics();
                assert_eq!(attempts.len(), 1);
                // Finite receipt retains the declared type; billing wins the
                // outward classification and precludes throttle authority.
                assert_eq!(
                    attempts[0].error_code,
                    if allowed && contradictory {
                        Some(ErrorCode::RateLimitExceeded)
                    } else {
                        None
                    }
                );
                assert!(!format!("{attempts:?}").contains("CANARY"));
                assert_eq!(state.grants, usize::from(admitted));
                assert_eq!(state.abandoned, 0);
                assert!(state.receipts.is_empty());
                assert!(!state.blocked);
                assert_eq!(
                    state.finishes,
                    if admitted {
                        vec![Feedback::Failure]
                    } else {
                        vec![]
                    }
                );
            }
        }
    }
}
#[tokio::test]
async fn s1_observed_error_budget_malformed_anthropic() {
    // Only Anthropic dispatches an explicitly named error with invalid JSON;
    // other providers' malformed JSON acceptance is deliberately S2, not S1.
    for surface in [Surface::Assembled, Surface::Incremental] {
        for admitted in [false, true] {
            for bytes in [64, 4096, 4097] {
                let raw = format!("{{{}", " ".repeat(bytes - 1));
                let (output, trace, state) = observe(
                    Leaf::Anthropic,
                    surface,
                    admitted,
                    format!("event: error\ndata: {raw}\n\n"),
                )
                .await;
                let error = outward(&output);
                assert_eq!(error.class, ProviderErrorClass::Server);
                assert_eq!(error.status, Some(502));
                assert!(!error.display.contains(&raw));
                assert_eq!(trace.attempt_diagnostics()[0].error_code, None);
                assert!(state.receipts.is_empty());
                assert!(!state.blocked);
                assert_eq!(state.grants, usize::from(admitted));
                assert_eq!(state.abandoned, 0);
                assert_eq!(
                    state.finishes,
                    if admitted {
                        vec![Feedback::Failure]
                    } else {
                        vec![]
                    }
                );
            }
        }
    }
}
