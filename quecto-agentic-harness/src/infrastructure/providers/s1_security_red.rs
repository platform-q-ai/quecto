//! S1 acceptance: real loopback provider errors preserve semantics without
//! exposing untrusted payloads. All matrix rows are collected before assertions.
//! This is an initial HTTP/SSE boundary family, not a complete sink inventory.
use std::sync::Arc;
use std::time::Duration;

use super::model_refusal::RefusalRecordingProvider;
use super::stream_idle_provider_tests::{Vendor, request, traced};
use crate::application::catalogue::CatalogueSnapshotStore;
use crate::application::providers::ports::LlmProvider;
use crate::domain::catalogue::ModelRef;
use crate::domain::error::DomainError;
use crate::domain::inference::events::request_observation::RequestTrace;
use crate::domain::inference::services::provider_error::{
    ProviderErrorClass as Class, classify_provider_error,
};
use crate::domain::inference::value_objects::provider::StreamEvent;
use crate::domain::message::Message;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

const LIMIT: Duration = Duration::from_secs(5);
const CANARIES: [&str; 6] = [
    "S1_SECRET_9f472",
    "sk-S1_TOKEN_281",
    "https://s1-private.invalid/path?key=793",
    "S1_MESSAGE_614",
    "S1_UNKNOWN_TYPE_839",
    "S1_UNKNOWN_CODE_257",
];

#[derive(Clone, Copy, Debug)]
enum Surface {
    Chat,
    Assembled,
    Incremental,
}
impl Surface {
    const ALL: [Self; 3] = [Self::Chat, Self::Assembled, Self::Incremental];
}

fn payload(kind: &str, code: &str, message: &str) -> String {
    serde_json::json!({"error": {"type": kind, "code": code, "message": message,
        "secret": CANARIES[0], "token": CANARIES[1], "url": CANARIES[2]},
        "detail": message})
    .to_string()
}
fn untrusted_message() -> String {
    CANARIES[..4].join(" ")
}

async fn mount(server: &MockServer, status: u16, body: String, sse: bool) {
    tokio::time::timeout(
        LIMIT,
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(status).set_body_raw(
                body,
                if sse {
                    "text/event-stream"
                } else {
                    "application/json"
                },
            ))
            .expect(1)
            .mount(server),
    )
    .await
    .expect("bounded fixture mount");
}

/// Return errors plus violations rather than asserting sanitation at the first row.
async fn invoke(
    provider: &dyn LlmProvider,
    surface: Surface,
    trace: &Arc<RequestTrace>,
) -> (Vec<String>, Vec<String>) {
    let messages = [Message::system("sys"), Message::user("hi")];
    tokio::time::timeout(LIMIT, async {
        match surface {
            Surface::Chat | Surface::Assembled => {
                let result = match surface {
                    Surface::Chat => provider.chat(request(&messages, trace)).await,
                    Surface::Assembled => provider.chat_stream(request(&messages, trace)).await,
                    Surface::Incremental => {
                        unreachable!("affirmatively selected whole-reply surfaces")
                    }
                };
                match result {
                    Err(DomainError::Provider(error)) => (vec![error], vec![]),
                    other => (
                        vec![],
                        vec![format!("expected provider error, got {other:?}")],
                    ),
                }
            }
            Surface::Incremental => {
                let mut rx = provider
                    .chat_stream_incremental(request(&messages, trace))
                    .await;
                let mut errors = vec![];
                let mut violations = vec![];
                while let Some(event) = rx.recv().await {
                    match event {
                        StreamEvent::Error(error) => errors.push(error),
                        other => violations.push(format!("non-error stream event: {other:?}")),
                    }
                }
                (errors, violations)
            }
        }
    })
    .await
    .expect("provider request and complete receiver drain must finish within five seconds")
}

fn safe(text: &str) -> bool {
    text.len() <= 4096 && CANARIES.iter().all(|canary| text.contains(canary) == false)
}

async fn inspect(
    server: &MockServer,
    trace: &RequestTrace,
    errors: &[String],
    expected: Class,
) -> Vec<String> {
    let mut failures = vec![];
    let sends = tokio::time::timeout(LIMIT, server.received_requests())
        .await
        .expect("bounded wire request inspection")
        .expect("wire recording enabled");
    if sends.len() == 1 {
    } else {
        failures.push(format!("physical POST count {}", sends.len()));
    }
    if errors.len() == 1 {
    } else {
        failures.push(format!("error count {}", errors.len()));
    }
    for error in errors {
        if safe(error) {
        } else {
            failures.push("outward error leaks canary or exceeds finite bound".into());
        }
        let class = classify_provider_error(&DomainError::Provider(error.clone()));
        if class == expected {
        } else {
            failures.push(format!("class {class:?}, expected {expected:?}"));
        }
    }
    let diagnostics = trace.attempt_diagnostics();
    // Some passive chat adapters expose no attempt diagnostics. Inspect every
    // available record without introducing an unrelated observability contract.
    if diagnostics.len() <= 1 {
    } else {
        failures.push(format!("diagnostic count {}", diagnostics.len()));
    }
    if diagnostics
        .iter()
        .all(|d| d.generated_text == false && d.generated_thinking == false)
    {
    } else {
        failures.push("generated output recorded on error".into());
    }
    if safe(&format!("{diagnostics:?}")) {
    } else {
        failures.push("trace diagnostics leak canary or exceed finite bound".into());
    }
    failures
}

fn report(label: String, failures: Vec<String>, rows: &mut Vec<(String, Vec<String>)>) {
    println!(
        "S1_ROW {label}: {}",
        if failures.is_empty() {
            "control-and-security-ok"
        } else {
            "missing-behavior"
        }
    );
    rows.push((label, failures));
}
fn conclude(rows: Vec<(String, Vec<String>)>) {
    let failures: Vec<_> = rows
        .iter()
        .filter(|(_, failures)| failures.is_empty() == false)
        .collect();
    println!(
        "S1_MATRIX exercised={} violating={}",
        rows.len(),
        failures.len()
    );
    assert!(
        failures.is_empty(),
        "missing secret-free finite provider boundary behavior: {failures:#?}"
    );
}

#[tokio::test]
async fn http_errors_are_secret_free_and_preserve_status_and_typed_classes() {
    // Given hostile HTTP bodies; When every public surface sends one POST;
    // Then only finite safe errors/diagnostics retain semantic classification.
    let message = untrusted_message();
    let cases = vec![
        (
            "server",
            500,
            payload(CANARIES[4], CANARIES[5], &message),
            Class::Server,
        ),
        (
            "rate-limit",
            429,
            payload(CANARIES[4], CANARIES[5], &message),
            Class::RateLimit,
        ),
        (
            "auth",
            401,
            payload(CANARIES[4], CANARIES[5], &message),
            Class::Auth,
        ),
        (
            "billing429",
            429,
            payload("billing_error", "insufficient_quota", &message),
            Class::Billing,
        ),
        (
            "invalid-request500",
            500,
            payload("invalid_request_error", CANARIES[5], &message),
            Class::Client,
        ),
        (
            "unknown-free-text500",
            500,
            payload(
                CANARIES[4],
                CANARIES[5],
                &format!("invalid_request_error insufficient_quota authentication_error {message}"),
            ),
            Class::Server,
        ),
        (
            "xai-capacity403",
            403,
            payload(
                CANARIES[4],
                CANARIES[5],
                &format!("xAI access to the chat endpoint is denied capacity temporary {message}"),
            ),
            Class::Auth,
        ),
        (
            "malformed-json500",
            500,
            format!("{{invalid-json {message} {} {}", CANARIES[4], CANARIES[5]),
            Class::Server,
        ),
        (
            "oversize500",
            500,
            payload(
                CANARIES[4],
                CANARIES[5],
                &format!("{} {message}", "x".repeat(5000)),
            ),
            Class::Server,
        ),
    ];
    let mut rows = vec![];
    for vendor in Vendor::ALL {
        for surface in Surface::ALL {
            for admitted in [false, true] {
                for (case, status, body, expected) in &cases {
                    let server = tokio::time::timeout(LIMIT, MockServer::start())
                        .await
                        .unwrap();
                    mount(&server, *status, body.clone(), false).await;
                    let provider = vendor.provider(server.uri(), admitted, LIMIT);
                    let trace = traced();
                    let (errors, mut failures) = invoke(provider.as_ref(), surface, &trace).await;
                    failures.extend(inspect(&server, &trace, &errors, expected.clone()).await);
                    report(
                        format!("http/{vendor:?}/{surface:?}/admitted={admitted}/{case}"),
                        failures,
                        &mut rows,
                    );
                }
            }
        }
    }
    assert_eq!(rows.len(), 162, "fixture matrix invariant");
    conclude(rows);
}

fn stream_body(vendor: Vendor, body: &str) -> String {
    match vendor {
        Vendor::OpenAi => format!("data: {body}\n\n"),
        Vendor::Anthropic => format!("event: error\ndata: {body}\n\n"),
        Vendor::Codex => {
            let error: serde_json::Value =
                serde_json::from_str(body).expect("valid SSE fixture JSON");
            format!(
                "data: {}\n\n",
                serde_json::json!({"type":"response.failed", "response":{"error":error["error"]}})
            )
        }
    }
}

#[tokio::test]
async fn sse_errors_are_secret_free_and_preserve_known_typed_semantics() {
    // Nonstreaming OpenAI/Anthropic chat does not accept SSE; explicitly omit it.
    let mut rows = vec![];
    for vendor in Vendor::ALL {
        let surfaces: &[Surface] = match vendor {
            Vendor::Codex => &Surface::ALL,
            Vendor::OpenAi | Vendor::Anthropic => &[Surface::Assembled, Surface::Incremental],
        };
        for &surface in surfaces {
            for admitted in [false, true] {
                for (case, kind, code, expected) in [
                    ("unknown", CANARIES[4], CANARIES[5], Class::Server),
                    ("server", "server_error", CANARIES[5], Class::Server),
                    (
                        "auth",
                        "authentication_error",
                        "invalid_api_key",
                        Class::Auth,
                    ),
                    (
                        "rate-limit",
                        "rate_limit_error",
                        "rate_limit_exceeded",
                        Class::RateLimit,
                    ),
                    (
                        "client",
                        "invalid_request_error",
                        "invalid_request_error",
                        Class::Client,
                    ),
                    (
                        "billing",
                        "billing_error",
                        "insufficient_quota",
                        Class::Billing,
                    ),
                ] {
                    let server = tokio::time::timeout(LIMIT, MockServer::start())
                        .await
                        .unwrap();
                    mount(
                        &server,
                        200,
                        stream_body(vendor, &payload(kind, code, &untrusted_message())),
                        true,
                    )
                    .await;
                    let provider = vendor.provider(server.uri(), admitted, LIMIT);
                    let trace = traced();
                    let (errors, mut failures) = invoke(provider.as_ref(), surface, &trace).await;
                    failures.extend(inspect(&server, &trace, &errors, expected).await);
                    report(
                        format!("sse/{vendor:?}/{surface:?}/admitted={admitted}/{case}"),
                        failures,
                        &mut rows,
                    );
                }
            }
        }
    }
    assert_eq!(rows.len(), 84, "reachable SSE matrix invariant");
    conclude(rows);
}

fn refusal_body(vendor: Vendor) -> (u16, String) {
    let message = untrusted_message();
    match vendor {
        Vendor::OpenAi => (
            404,
            payload("invalid_request_error", "model_not_found", &message),
        ),
        Vendor::Anthropic => (
            404,
            payload(
                "not_found_error",
                CANARIES[5],
                &format!("model: mini {message}"),
            ),
        ),
        Vendor::Codex => (
            400,
            payload(
                CANARIES[4],
                CANARIES[5],
                &format!(
                    "The 'mini' model is not supported when using Codex with a ChatGPT account. {message}"
                ),
            ),
        ),
    }
}

fn success_body(vendor: Vendor, surface: Surface) -> (String, bool) {
    match (vendor, surface) {
        (Vendor::OpenAi, Surface::Chat) => (r#"{"choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}"#.into(), false),
        (Vendor::Anthropic, Surface::Chat) => (r#"{"content":[{"type":"text","text":"ok"}],"stop_reason":"end_turn"}"#.into(), false),
        (Vendor::OpenAi, _) => ("data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n".into(), true),
        (Vendor::Anthropic, _) => ("event: content_block_start\ndata: {\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"ok\"}}\n\nevent: content_block_stop\ndata: {\"index\":0}\n\nevent: message_stop\ndata: {}\n\n".into(), true),
        (Vendor::Codex, _) => ("data: {\"type\":\"response.output_text.delta\",\"delta\":\"ok\"}\n\ndata: {\"type\":\"response.completed\",\"response\":{}}\n\n".into(), true),
    }
}

async fn succeed(provider: &dyn LlmProvider, surface: Surface) -> bool {
    let messages = [Message::system("sys"), Message::user("hi")];
    let trace = traced();
    tokio::time::timeout(LIMIT, async {
        match surface {
            Surface::Chat => provider.chat(request(&messages, &trace)).await.is_ok(),
            Surface::Assembled => provider
                .chat_stream(request(&messages, &trace))
                .await
                .is_ok(),
            Surface::Incremental => {
                let mut rx = provider
                    .chat_stream_incremental(request(&messages, &trace))
                    .await;
                let mut done = false;
                let mut error = false;
                while let Some(event) = rx.recv().await {
                    match event {
                        StreamEvent::Done(_) => done = true,
                        StreamEvent::Error(_) => error = true,
                        _ => {}
                    }
                }
                done && error == false
            }
        }
    })
    .await
    .expect("bounded success control and drain")
}

#[tokio::test]
async fn catalogue_holds_only_known_refusals_with_safe_reasons_and_success_clears_them() {
    // Given the real refusal decorator/store; When hostile known/unknown bodies
    // reach a vendor; Then safe holds are selective, and normal success clears.
    let mut rows = vec![];
    for vendor in Vendor::ALL {
        for surface in Surface::ALL {
            for admitted in [false, true] {
                let (known_status, known_body) = refusal_body(vendor);
                for (case, status, body, should_hold) in [
                    ("known-model", known_status, known_body.clone(), true),
                    ("wrong-status", 500, known_body.clone(), false),
                    ("auth", 401, known_body, false),
                    (
                        "unknown",
                        404,
                        payload(CANARIES[4], CANARIES[5], &untrusted_message()),
                        false,
                    ),
                    (
                        "file-not-model",
                        404,
                        payload(
                            "not_found_error",
                            "file_not_found",
                            &format!("file: missing {}", untrusted_message()),
                        ),
                        false,
                    ),
                    (
                        "malformed",
                        404,
                        format!("{{invalid-json {}", untrusted_message()),
                        false,
                    ),
                    (
                        "oversize",
                        404,
                        payload(
                            CANARIES[4],
                            CANARIES[5],
                            &format!("{} {}", "x".repeat(5000), untrusted_message()),
                        ),
                        false,
                    ),
                ] {
                    let server = tokio::time::timeout(LIMIT, MockServer::start())
                        .await
                        .unwrap();
                    mount(&server, status, body, false).await;
                    let leaf = vendor.provider(server.uri(), admitted, LIMIT);
                    let reference = ModelRef::parse(leaf.name(), "gpt-test")
                        .expect("real leaf route identifier");
                    let store = Arc::new(CatalogueSnapshotStore::empty());
                    let provider = RefusalRecordingProvider::new(
                        leaf,
                        store.clone(),
                        Duration::from_secs(3600),
                    );
                    let trace = traced();
                    let (errors, mut failures) = invoke(&provider, surface, &trace).await;
                    // OpenAI's known-model fixture declares a typed client error,
                    // which stays Client even under a wrong 500 status (S1).
                    let expected = match (vendor, case) {
                        (Vendor::OpenAi, "wrong-status") => Class::Client,
                        _ => Class::from_status(status),
                    };
                    failures.extend(inspect(&server, &trace, &errors, expected).await);
                    let reason = store.refusal(&reference);
                    if reason.is_some() == should_hold {
                    } else {
                        failures.push(format!(
                            "hold presence {}, expected {should_hold}",
                            reason.is_some()
                        ));
                    }
                    if reason
                        .as_deref()
                        .is_none_or(|reason| safe(reason) && reason.trim().is_empty() == false)
                    {
                    } else {
                        failures.push("refusal reason exposes untrusted fields or is empty".into());
                    }
                    if should_hold {
                        tokio::time::timeout(LIMIT, server.reset())
                            .await
                            .expect("bounded fixture reset");
                        let (body, sse) = success_body(vendor, surface);
                        mount(&server, 200, body, sse).await;
                        if succeed(&provider, surface).await {
                        } else {
                            failures.push("normal success fixture did not succeed".into());
                        }
                        if store.refusal(&reference).is_none() {
                        } else {
                            failures.push("success failed to clear hold".into());
                        }
                        let sends = tokio::time::timeout(LIMIT, server.received_requests())
                            .await
                            .unwrap()
                            .unwrap();
                        if sends.len() == 1 {
                        } else {
                            failures.push(format!("success POST count {}", sends.len()));
                        }
                    }
                    report(
                        format!("refusal/{vendor:?}/{surface:?}/admitted={admitted}/{case}"),
                        failures,
                        &mut rows,
                    );
                }
            }
        }
    }
    assert_eq!(rows.len(), 126, "refusal matrix invariant");
    conclude(rows);
}
