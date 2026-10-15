//! Actual-provider contradictory-envelope and refusal regressions.
use super::fixture::*;
use crate::inference_admission_feedback_transport::fixture::{Reply as WireReply, Server};
use quecto::domain::inference::services::provider_error::ProviderErrorClass;
use std::sync::Arc;
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

#[tokio::test]
async fn remediation_actual_openai_contradictory_envelopes_both_directions() {
    let mut rows = 0;
    for selected_throttle in [false, true] {
        for surface in [Surface::Assembled, Surface::Incremental] {
            for admitted in [false, true] {
                for header in [false, true] {
                    let selected = if selected_throttle {
                        "rate_limit_error"
                    } else {
                        "invalid_request_error"
                    };
                    let ignored = if selected_throttle {
                        "invalid_request_error"
                    } else {
                        "rate_limit_error"
                    };
                    let raw = serde_json::json!({"type":"response.failed","response":{"error":{"type":selected}},"error":{"type":ignored}});
                    let server = Server::start(WireReply {
                        status: 200,
                        headers: format!(
                            "Content-Type: text/event-stream\r\n{}",
                            if header { "Retry-After: 2\r\n" } else { "" }
                        ),
                        body: format!("data: {raw}\n\n"),
                        stalled: false,
                    })
                    .await;
                    let gate = Arc::new(Gate::default());
                    let provider =
                        Leaf::OpenAi.provider(&server.url, admitted.then(|| gate.clone()));
                    let output =
                        bounded(invoke_request(provider.as_ref(), surface, request())).await;
                    let error = outward(&output);
                    assert_eq!(
                        error.status,
                        Some(if selected_throttle { 429 } else { 400 }),
                        "{surface:?}/{admitted}/{header}/{selected_throttle}: {error:?}"
                    );
                    assert_eq!(
                        error.class,
                        if selected_throttle {
                            ProviderErrorClass::RateLimit
                        } else {
                            ProviderErrorClass::Client
                        }
                    );
                    let state = gate.snapshot();
                    assert_eq!(
                        state.receipts.len(),
                        usize::from(admitted && selected_throttle),
                        "{state:?}"
                    );
                    rows += 1;
                }
            }
        }
    }
    assert_eq!(rows, 16);
}

#[tokio::test]
async fn remediation_actual_codex_mixed_and_root_only_refusal() {
    let detail = "The 'm' model is not supported when using Codex with a ChatGPT account.";
    let mut rows = 0;
    for leaf in [Leaf::Responses, Leaf::OAuth] {
        for surface in [Surface::Chat, Surface::Assembled, Surface::Incremental] {
            for admitted in [false, true] {
                for (body, refused) in [
                    (serde_json::json!({"detail":detail}), true),
                    (serde_json::json!({"detail":detail,"error":{}}), false),
                    (
                        serde_json::json!({"detail":"unrelated","error":{"message":detail}}),
                        true,
                    ),
                ] {
                    let server = Server::start(WireReply {
                        status: 400,
                        headers: String::new(),
                        body: body.to_string(),
                        stalled: false,
                    })
                    .await;
                    let gate = Arc::new(Gate::default());
                    let provider = leaf.provider(&server.url, admitted.then(|| gate.clone()));
                    let output =
                        bounded(invoke_request(provider.as_ref(), surface, request())).await;
                    let error = outward(&output);
                    let domain =
                        quecto::domain::error::DomainError::Provider(error.display.clone());
                    assert_eq!(
                        quecto::domain::inference::services::provider_error::model_refusal(&domain)
                            .is_some(),
                        refused,
                        "{leaf:?}/{surface:?}/{admitted}: {error:?}"
                    );
                    rows += 1;
                }
            }
        }
    }
    assert_eq!(rows, 36);
}

#[tokio::test]
async fn remediation_actual_validated_header_authority_survives_body_billing() {
    use quecto::domain::admission::value_objects::inference_admission::ThrottleFeedback;
    let mut rows = 0;
    for surface in [Surface::Chat, Surface::Assembled, Surface::Incremental] {
        for admitted in [false, true] {
            let server=Server::start(WireReply {
                status:429,headers:"Retry-After: 2\r\n".into(),
                body:r#"{"error":{"type":"insufficient_quota"},"response":{"error":{"type":"rate_limit_error"}}}"#.into(),stalled:false,
            }).await;
            let gate = Arc::new(Gate::default());
            let provider = Leaf::OpenAi.provider(&server.url, admitted.then(|| gate.clone()));
            let output = bounded(invoke_request(provider.as_ref(), surface, request())).await;
            assert_eq!(outward(&output).class, ProviderErrorClass::Billing);
            let state = gate.snapshot();
            if admitted {
                assert!(
                    matches!(state.receipts.as_slice(), [ThrottleFeedback::Until(_)]),
                    "{state:?}"
                );
            } else {
                assert!(state.receipts.is_empty());
            }
            rows += 1;
        }
    }
    assert_eq!(rows, 6);
}
