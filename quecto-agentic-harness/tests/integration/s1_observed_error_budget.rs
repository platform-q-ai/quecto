//! Actual leaf wrappers, not bare SSE handlers. Nine SSE surfaces times two
//! admission modes: OpenAI incremental; Anthropic assembled/incremental;
//! Responses and OAuth chat/assembled/incremental. Other chat surfaces consume
//! non-SSE JSON and OpenAI assembled uses its JSON chat path, so are not rows. Eighteen admission rows are exercised.
use super::fixture::*;
use crate::inference_admission_feedback_transport::fixture::{Reply as WireReply, Server};
use quecto::domain::admission::value_objects::inference_admission::Feedback;
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
        Leaf::OpenAi | Leaf::Anthropic => value["error"]["message"] = "x".repeat(padding).into(),
        _ => value["response"]["error"]["message"] = "x".repeat(padding).into(),
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
    let (output, trace, state) =
        observe(leaf, surface, admitted, envelope(leaf, bytes, billing)).await;
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
    eprintln!("ROW {leaf:?}/{surface:?}/{admitted}/{billing}/{bytes} permit={state:?}");
    let attempts = trace.attempt_diagnostics();
    assert_eq!(attempts.len(), 1);
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
        state.receipts.len(),
        usize::from(admitted && allowed && !billing)
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

#[tokio::test]
async fn s1_observed_error_budget_large_output_and_terminal() {
    for leaf in [Leaf::OpenAi, Leaf::Anthropic, Leaf::Responses, Leaf::OAuth] {
        for admitted in [false, true] {
            let text = "t".repeat(6000);
            let (delta, terminal) = match leaf {
                Leaf::OpenAi => (
                    format!(
                        "data: {}",
                        serde_json::json!({"choices":[{"delta":{"content":text}}]})
                    ),
                    "data: [DONE]".to_owned(),
                ),
                _ => (leaf.delta(&text), leaf.terminal().to_owned()),
            };
            let body = format!("{delta}\n\n{terminal}\n\n{}", envelope(leaf, 4096, false));
            let (output, trace, state) = observe(leaf, Surface::Incremental, admitted, body).await;
            let Observation::Events(events) = output else {
                panic!("expected events")
            };
            assert!(
                events
                    .iter()
                    .any(|e| matches!(e, Event::Other(s) if s.contains(&text)))
            );
            assert!(events.iter().all(|e| matches!(e, Event::Other(_))));
            assert_eq!(trace.attempt_diagnostics()[0].error_code, None);
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

#[tokio::test]
async fn s1_observed_error_budget_large_tool() {
    for leaf in [Leaf::OpenAi, Leaf::Anthropic, Leaf::Responses, Leaf::OAuth] {
        for admitted in [false, true] {
            let arguments = "a".repeat(6000);
            let data = match leaf {
                Leaf::OpenAi => {
                    serde_json::json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call","type":"function","function":{"name":"fixture","arguments":arguments}}]}}]})
                }
                Leaf::Anthropic => {
                    serde_json::json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"call","name":"fixture","input":{"arg":arguments}}})
                }
                _ => {
                    serde_json::json!({"type":"response.output_item.added","item":{"type":"function_call","id":"call","call_id":"call","name":"fixture","arguments":arguments}})
                }
            };
            let body = format!(
                "{}data: {data}\n\n{}\n\n",
                if matches!(leaf, Leaf::Anthropic) {
                    "event: content_block_start\n"
                } else {
                    ""
                },
                if matches!(leaf, Leaf::OpenAi) {
                    "data: [DONE]"
                } else {
                    leaf.terminal()
                }
            );
            let (output, trace, state) = observe(leaf, Surface::Incremental, admitted, body).await;
            let Observation::Events(events) = output else {
                panic!("expected events")
            };
            assert!(events.iter().all(|e| matches!(e, Event::Other(_))));
            let attempts = trace.attempt_diagnostics();
            assert_eq!(attempts.len(), 1);
            assert!(attempts[0].generated_tool_call);
            assert_eq!(attempts[0].error_code, None);
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
