//! Actual leaf wrappers: ten SSE surfaces times two admission modes = 20 cells.
//! OpenAI assembled/incremental; Anthropic assembled/incremental;
//! Responses API-key and OAuth chat/assembled/incremental, passive/admitted.
//! Non-SSE OpenAI and Anthropic chat are not SSE boundary cells.
use super::fixture::*;
use crate::inference_admission_feedback_transport::fixture::{Reply as WireReply, Server};
use quecto::domain::admission::value_objects::inference_admission::{Feedback, ThrottleFeedback};
use quecto::domain::inference::events::request_observation::RequestTrace;
use quecto::domain::inference::services::provider_error::ProviderErrorClass;
use quecto::domain::inference::value_objects::attempt_diagnostics::ErrorCode;
use std::sync::Arc;

fn envelope(leaf: Leaf, bytes: usize, billing: bool) -> String {
    let error = serde_json::json!({"type": if billing {"invalid_request_error"} else {"rate_limit_error"},
        "code": if billing {"insufficient_quota"} else {"rate_limit_exceeded"}, "message":""});
    let mut value = match leaf {
        Leaf::OpenAi | Leaf::Anthropic => serde_json::json!({"type":"error","error":error}),
        _ => serde_json::json!({"type":"response.failed","response":{"error":error}}),
    };
    let padding = bytes.checked_sub(value.to_string().len()).unwrap();
    match leaf {
        Leaf::OpenAi | Leaf::Anthropic => {
            value["error"]["message"] = format!("CANARY{}", "x".repeat(padding - 6)).into()
        }
        _ => {
            value["response"]["error"]["message"] =
                format!("CANARY{}", "x".repeat(padding - 6)).into()
        }
    }
    let raw = value.to_string();
    assert_eq!(raw.len(), bytes);
    format!(
        "{}data: {raw}\n\n",
        if matches!(leaf, Leaf::Anthropic) {
            "event: error\n"
        } else {
            ""
        }
    )
}
async fn observe(
    leaf: Leaf,
    surface: Surface,
    admitted: bool,
    body: String,
) -> (Observation, Arc<RequestTrace>, Snapshot) {
    let server = Server::start(WireReply {
        status: 200,
        headers: "Content-Type: text/event-stream\r\n".into(),
        body,
        stalled: false,
    })
    .await;
    let gate = Arc::new(Gate::default());
    let provider = leaf.provider(&server.url, admitted.then(|| gate.clone()));
    let trace = Arc::new(RequestTrace::default());
    trace.start();
    let mut req = request();
    req.trace = Some(trace.clone());
    let output = bounded(invoke_request(provider.as_ref(), surface, req)).await;
    (output, trace, gate.snapshot())
}
fn outward(output: &Observation) -> &Error {
    match output {
        Observation::Error(error) => error,
        Observation::Events(events) => {
            let errors: Vec<_> = events
                .iter()
                .filter_map(|event| match event {
                    Event::Error { classified, .. } => Some(classified),
                    _ => None,
                })
                .collect();
            assert_eq!(errors.len(), 1);
            errors[0]
        }
        other => panic!("expected finite error: {other:?}"),
    }
}
async fn boundary(leaf: Leaf, surface: Surface, admitted: bool, billing: bool, bytes: usize) {
    boundary_body(
        leaf,
        surface,
        admitted,
        billing,
        bytes,
        envelope(leaf, bytes, billing),
    )
    .await;
}
async fn boundary_body(
    leaf: Leaf,
    surface: Surface,
    admitted: bool,
    billing: bool,
    bytes: usize,
    body: String,
) {
    let (output, trace, state) = observe(leaf, surface, admitted, body).await;
    let error = outward(&output);
    let allowed = bytes == 4096;
    assert_eq!(
        error.class,
        if allowed && !billing {
            ProviderErrorClass::RateLimit
        } else if allowed {
            ProviderErrorClass::Billing
        } else {
            ProviderErrorClass::Server
        },
        "{leaf:?}/{surface:?}/{admitted}/{billing}/{bytes}"
    );
    assert_eq!(
        error.status,
        Some(if allowed && !billing {
            429
        } else if allowed {
            402
        } else {
            502
        })
    );
    assert!(error.display.contains("refusal=none"), "{}", error.display);
    assert!(!error.display.contains("CANARY"));
    eprintln!("ROW {leaf:?}/{surface:?}/{admitted}/{billing}/{bytes} permit={state:?}");
    let attempts = trace.attempt_diagnostics();
    assert_eq!(attempts.len(), 1);
    assert!(!format!("{attempts:?}").contains("CANARY"));
    assert_eq!(
        attempts[0].error_code,
        if allowed {
            Some(if billing {
                ErrorCode::InvalidRequestError
            } else {
                ErrorCode::RateLimitExceeded
            })
        } else {
            None
        }
    );
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
    assert_eq!(
        state.receipts,
        if admitted && allowed && !billing {
            vec![ThrottleFeedback::NoHint { jitter: u64::MAX }]
        } else {
            vec![]
        }
    );
    // This captured permit closes its recording gate on feedback:
    // prove cooldown-trigger neutrality, not production expiry policy.
    assert_eq!(state.blocked, admitted && allowed && !billing);
}
macro_rules! row {
    ($name:ident, $leaf:ident, $surface:ident) => {
        mod $name {
            use super::*;
            macro_rules! case {
                ($test:ident, $admitted:expr, $billing:expr, $bytes:expr) => {
                    #[tokio::test]
                    async fn $test() {
                        boundary(Leaf::$leaf, Surface::$surface, $admitted, $billing, $bytes).await;
                    }
                };
            }
            case!(passive_throttle_4096, false, false, 4096);
            case!(passive_throttle_4097, false, false, 4097);
            case!(passive_billing_4096, false, true, 4096);
            case!(passive_billing_4097, false, true, 4097);
            case!(admitted_throttle_4096, true, false, 4096);
            case!(admitted_throttle_4097, true, false, 4097);
            case!(admitted_billing_4096, true, true, 4096);
            case!(admitted_billing_4097, true, true, 4097);
        }
    };
}
row!(s1_observed_error_budget_openai_assembled, OpenAi, Assembled);
row!(
    s1_observed_error_budget_openai_incremental,
    OpenAi,
    Incremental
);
row!(
    s1_observed_error_budget_anthropic_assembled,
    Anthropic,
    Assembled
);
row!(
    s1_observed_error_budget_anthropic_incremental,
    Anthropic,
    Incremental
);
row!(s1_observed_error_budget_responses_chat, Responses, Chat);
row!(
    s1_observed_error_budget_responses_assembled,
    Responses,
    Assembled
);
row!(
    s1_observed_error_budget_responses_incremental,
    Responses,
    Incremental
);
row!(s1_observed_error_budget_oauth_chat, OAuth, Chat);
row!(s1_observed_error_budget_oauth_assembled, OAuth, Assembled);
row!(
    s1_observed_error_budget_oauth_incremental,
    OAuth,
    Incremental
);

#[path = "s1_observed_error_controls.rs"]
mod error_controls;
#[path = "s1_observed_output_budget.rs"]
mod output_budget;

fn cells() -> impl Iterator<Item = (Leaf, Surface, bool)> {
    [
        (Leaf::OpenAi, Surface::Assembled),
        (Leaf::OpenAi, Surface::Incremental),
        (Leaf::Anthropic, Surface::Assembled),
        (Leaf::Anthropic, Surface::Incremental),
        (Leaf::Responses, Surface::Chat),
        (Leaf::Responses, Surface::Assembled),
        (Leaf::Responses, Surface::Incremental),
        (Leaf::OAuth, Surface::Chat),
        (Leaf::OAuth, Surface::Assembled),
        (Leaf::OAuth, Surface::Incremental),
    ]
    .into_iter()
    .flat_map(|(leaf, surface)| {
        [false, true]
            .into_iter()
            .map(move |admitted| (leaf, surface, admitted))
    })
}
#[tokio::test]
async fn s1_observed_error_budget_raw_whitespace() {
    for (leaf, surface, admitted) in cells() {
        for billing in [false, true] {
            for bytes in [4096, 4097] {
                for suffix in [" ", "\t", "\r "] {
                    let body = envelope(leaf, bytes - suffix.len(), billing);
                    let body = body.replace("\n\n", &format!("{suffix}\n\n"));
                    boundary_body(leaf, surface, admitted, billing, bytes, body).await;
                }
            }
        }
    }
}
#[tokio::test]
async fn s1_observed_error_budget_utf8() {
    for (leaf, surface, admitted) in cells() {
        for bytes in [4096, 4097] {
            let body = envelope(leaf, bytes, false).replacen("xx", "é", 1);
            boundary_body(leaf, surface, admitted, false, bytes, body).await;
        }
    }
}

#[tokio::test]
async fn s1_observed_error_budget_codex_success_metadata() {
    // 2 credential paths x 3 surfaces x 2 admission modes x 2 error locations = 24.
    for (leaf, surface, admitted) in
        cells().filter(|(leaf, _, _)| matches!(leaf, Leaf::Responses | Leaf::OAuth))
    {
        for nested in [false, true] {
            for size in [8, 6000] {
                let text = "t".repeat(size);
                let mut value =
                    serde_json::json!({"type":"response.output_text.delta","delta":text});
                let error = serde_json::json!({"type":"rate_limit_error","code":"rate_limit_exceeded","message":"CANARY_METADATA"});
                if nested {
                    value["response"] = serde_json::json!({"error":error});
                } else {
                    value["error"] = error;
                }
                let body = format!("data: {value}\n\n{}", leaf.terminal());
                let (output, trace, state) = observe(leaf, surface, admitted, body).await;
                assert!(
                    format!("{output:?}").contains(&text),
                    "{leaf:?}/{surface:?}/{admitted}/{nested}/{size}: {output:?}"
                );
                assert_eq!(
                    trace.attempt_diagnostics()[0].error_code,
                    None,
                    "{leaf:?}/{surface:?}/{admitted}/{nested}/{size}"
                );
                assert!(state.receipts.is_empty());
                assert!(!state.blocked);
                assert_eq!(
                    state.finishes,
                    if admitted {
                        vec![Feedback::Success]
                    } else {
                        vec![]
                    }
                );
            }
        }
    }
}

#[tokio::test]
async fn s1_observed_error_budget_crlf_framing() {
    for (leaf, surface, admitted) in cells() {
        for bytes in [4096, 4097] {
            let body = envelope(leaf, bytes - 1, false)
                .replace("\n\n", " \n\n")
                .replace('\n', "\r\n");
            boundary_body(leaf, surface, admitted, false, bytes, body).await;
        }
    }
}

#[tokio::test]
async fn s1_semantic_authority_http_root_and_nested() {
    let mut failures = Vec::new();
    let mut count = 0;
    for leaf in [Leaf::OpenAi, Leaf::Anthropic, Leaf::Responses, Leaf::OAuth] {
        for surface in [Surface::Chat, Surface::Assembled, Surface::Incremental] {
            for admitted in [false, true] {
                for nested in [false, true] {
                    for (status, billing) in [(429, true), (500, false)] {
                        let error = serde_json::json!({"code":if billing {"insufficient_quota"} else {"rate_limit_exceeded"},"message":"CANARY_ROOT"});
                        let body = if nested {
                            serde_json::json!({"error":error})
                        } else {
                            error
                        }
                        .to_string();
                        let server = Server::start(WireReply {
                            status,
                            headers: String::new(),
                            body,
                            stalled: false,
                        })
                        .await;
                        let gate = Arc::new(Gate::default());
                        let provider = leaf.provider(&server.url, admitted.then(|| gate.clone()));
                        let trace = Arc::new(RequestTrace::default());
                        trace.start();
                        let mut req = request();
                        req.trace = Some(trace.clone());
                        let output = bounded(invoke_request(provider.as_ref(), surface, req)).await;
                        let error = outward(&output);
                        let state = gate.snapshot();
                        assert_eq!(
                            error.class,
                            if billing {
                                ProviderErrorClass::Billing
                            } else {
                                ProviderErrorClass::Server
                            }
                        );
                        assert_eq!(error.status, Some(status));
                        let attempts = trace.attempt_diagnostics();
                        assert!(attempts.len() <= 1);
                        if admitted {
                            assert_eq!(attempts.len(), 1);
                        }
                        for attempt in attempts {
                            assert_eq!(
                                attempt.error_code,
                                Some(if billing {
                                    ErrorCode::InsufficientQuota
                                } else {
                                    ErrorCode::RateLimitExceeded
                                })
                            );
                        }
                        assert!(
                            !format!("{output:?}{:?}", trace.attempt_diagnostics())
                                .contains("CANARY")
                        );
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
                        if billing || !nested {
                            if !state.receipts.is_empty() || state.blocked {
                                failures.push(format!("{leaf:?}/{surface:?}/{admitted}/{nested}/{status}: unexpected body throttle"));
                            }
                        } else {
                            assert_eq!(state.blocked, admitted);
                            assert_eq!(state.receipts.len(), usize::from(admitted));
                        }
                        count += 1;
                    }
                }
            }
        }
    }
    assert_eq!(count, 96);
    assert!(
        failures.is_empty(),
        "{} cells: {failures:?}",
        failures.len()
    );
}

#[tokio::test]
async fn s1_semantic_authority_raw_json_whitespace() {
    let mut failures = Vec::new();
    let mut count = 0;
    for (leaf, surface, admitted) in cells() {
        // LF is carried by multiline data, so it remains inside RAW JSON.
        for (suffix, valid) in [
            (" ", true),
            ("\t", true),
            ("\r", true),
            ("\n", true),
            ("\u{a0}", false),
            ("\u{2003}", false),
            ("\u{202f}", false),
        ] {
            let body = envelope(leaf, 128, true).replace("\n\n", &format!("{suffix}\n\n"));
            let (output, trace, state) = observe(leaf, surface, admitted, body).await;
            let error = outward(&output);
            assert!(!format!("{output:?}{:?}", trace.attempt_diagnostics()).contains("CANARY"));
            let attempts = trace.attempt_diagnostics();
            assert_eq!(attempts.len(), 1);
            assert_eq!(
                attempts[0].error_code,
                valid.then_some(ErrorCode::InvalidRequestError)
            );
            assert_eq!(attempts[0].parse_errors, u32::from(!valid));
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
            assert!(state.receipts.is_empty());
            assert!(!state.blocked);
            if valid {
                assert_eq!(error.class, ProviderErrorClass::Billing);
                assert_eq!(error.status, Some(402));
            } else if error.class == ProviderErrorClass::Billing || error.status == Some(402) {
                failures.push(format!(
                    "{leaf:?}/{surface:?}/{admitted}/{suffix:?}: malformed RAW granted billing"
                ));
            } else {
                // Legacy framing may ignore malformed JSON; either unknown or
                // finite server failure is permitted, never body semantics.
                assert!(
                    matches!(
                        error.class,
                        ProviderErrorClass::Server
                            | ProviderErrorClass::Unknown
                            | ProviderErrorClass::EmptyStream
                            | ProviderErrorClass::Network
                    ),
                    "{leaf:?}/{surface:?}/{admitted}: class={:?} status={:?}",
                    error.class,
                    error.status
                );
                assert!(matches!(error.status, None | Some(502)));
            }
            count += 1;
        }
    }
    assert_eq!(count, 140);
    assert!(
        failures.is_empty(),
        "{} cells: {failures:?}",
        failures.len()
    );
}

#[tokio::test]
async fn s1_semantic_authority_active_oversized_parse() {
    // Small serialized value + large legal raw padding falsifies a guard that
    // admits active parsing but later relies on serialized-value size alone.
    for (leaf, surface, admitted) in cells() {
        let body =
            envelope(leaf, 128, true).replace("\n\n", &format!("{}\n\n", " ".repeat(4097 - 128)));
        boundary_body(leaf, surface, admitted, true, 4097, body).await;
    }
}
