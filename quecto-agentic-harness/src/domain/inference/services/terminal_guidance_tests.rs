//! #931 and #2459: the guidance a terminal provider failure gets names its
//! class and says what happened: a retry only when one was made, "could
//! not reach" only when no response arrived, and, after output streamed,
//! that the reply was not replayed and continuing is safe.
use super::*;

fn facts(attempts: u32, output: Output, response: Response) -> AttemptFacts {
    AttemptFacts {
        attempts,
        output,
        response,
    }
}

/// One attempt, no output, nothing recorded about a response.
const PLAIN: AttemptFacts = AttemptFacts {
    attempts: 1,
    output: Output::NotShown,
    response: Response::Unknown,
};

fn explained(message: &str, facts: AttemptFacts) -> String {
    match explain_terminal_failure(DomainError::Provider(message.to_string()), facts) {
        DomainError::Provider(m) => m,
        other => panic!("expected Provider error, got {other:?}"),
    }
}

fn guidance(message: &str, facts: AttemptFacts) -> String {
    let out = explained(message, facts);
    out.strip_prefix(&format!("{message}\n\n"))
        .unwrap_or_else(|| panic!("guidance appended after the message: {out}"))
        .to_string()
}

const CUT_SHORT: &str =
    "provider error: OpenAI SSE stream ended without completion: connection closed before [DONE]";

#[test]
fn a_stream_cut_off_after_output_was_not_retried_and_may_continue() {
    let out = guidance(CUT_SHORT, facts(1, Output::Shown, Response::Received));
    assert_eq!(
        out,
        "Network: the provider closed the connection partway through the reply. It was not \
         retried, because a reply that has already streamed output is never replayed; continuing \
         or re-prompting is safe."
    );
    assert!(!out.contains("It was retried"));
    assert!(!out.contains("could not reach"));
}

#[test]
fn output_shown_never_claims_a_retry_for_any_retryable_class() {
    for message in [
        CUT_SHORT,
        "HTTP 503 OpenAI stream error: overloaded",
        "provider error (429): rate limit exceeded",
    ] {
        for attempts in [1, 3] {
            let out = guidance(message, facts(attempts, Output::Shown, Response::Received));
            assert!(out.contains("It was not retried"), "{message}: {out}");
            assert!(out.contains("never replayed"), "{message}: {out}");
            assert!(!out.contains("It was retried"), "{message}: {out}");
        }
    }
}

#[test]
fn an_unreachable_provider_after_retries_says_so_with_the_attempt_count() {
    let out = guidance(
        "HTTP error: error sending request: connection refused",
        facts(3, Output::NotShown, Response::NotReceived),
    );
    assert_eq!(
        out,
        "Network: could not reach the provider (connection/timeout). It was retried (3 attempts) \
         and still failed — check connectivity and retry later."
    );
}

#[test]
fn could_not_reach_is_said_only_when_no_response_arrived() {
    for response in [Response::Received, Response::Unknown] {
        let out = guidance(CUT_SHORT, facts(3, Output::NotShown, response));
        assert!(!out.contains("could not reach"), "{response:?}: {out}");
        assert!(out.contains("retried (3 attempts)"), "{out}");
    }
    let once = guidance(CUT_SHORT, facts(1, Output::NotShown, Response::NotReceived));
    assert!(once.contains("could not reach") && once.contains("It was not retried"));
}

#[test]
fn server_and_rate_limit_claim_a_retry_only_after_more_than_one_attempt() {
    let server =
        "HTTP 529 from Anthropic: {\"type\":\"error\",\"error\":{\"type\":\"overloaded_error\"}}";
    let rate = "provider error (429): rate limit exceeded";
    assert_eq!(
        guidance(server, PLAIN),
        "Server/overload: the provider is overloaded or returned a 5xx error. It was not retried \
         — retry later."
    );
    assert_eq!(
        guidance(server, facts(4, Output::NotShown, Response::Received)),
        "Server/overload: the provider is overloaded or returned a 5xx error. It was retried (4 \
         attempts) and still failed — retry later."
    );
    assert_eq!(
        guidance(rate, PLAIN),
        "Rate limit: the provider throttled the request. It was not retried — wait and retry \
         later, or reduce request frequency."
    );
    assert_eq!(
        guidance(rate, facts(2, Output::NotShown, Response::Received)),
        "Rate limit: the provider throttled the request. It was retried with backoff (2 attempts) \
         and still failed — wait and retry later, or reduce request frequency."
    );
}

#[test]
fn zero_recorded_attempts_read_as_one() {
    let out = guidance(CUT_SHORT, facts(0, Output::NotShown, Response::Unknown));
    assert!(out.contains("It was not retried"), "{out}");
}

#[test]
fn the_codex_507_retry_buffer_limit_gets_server_guidance() {
    let message = "HTTP 507 from Codex: exceeded request buffer limit while retrying upstream";
    assert!(
        guidance(message, facts(3, Output::NotShown, Response::Received)).starts_with(
            "Server/overload: the provider is overloaded or returned a 5xx error. It was retried (3"
        )
    );
}

#[test]
fn a_client_error_is_passed_through_unchanged() {
    let input = "provider error (400): invalid_request_error";
    assert_eq!(explained(input, PLAIN), input);
}

#[test]
fn a_non_provider_error_is_returned_as_it_is() {
    let err = explain_terminal_failure(DomainError::Tool("boom".into()), PLAIN);
    assert!(matches!(err, DomainError::Tool(m) if m == "boom"));
}

#[test]
fn synthetic_empty_stream_guidance_does_not_claim_http_failure_or_overload() {
    let out = explained(
        "stream completed without assistant output: synthetic=empty_stream",
        PLAIN,
    );
    assert!(out.contains("synthetic classification"));
    assert!(out.contains("no HTTP failure or overload is established"));
    assert!(!out.contains("provider is overloaded"));
}

#[test]
fn a_context_limit_error_gets_the_limit_hint_once() {
    let out = explained("maximum context length exceeded: too many tokens", PLAIN);
    assert!(out.contains("Context/output limit"));
    assert_eq!(explained(&out, PLAIN), out, "appended once");
    let already = "Context/output limit: too many tokens in context";
    assert_eq!(
        explained(already, PLAIN)
            .matches("Context/output limit")
            .count(),
        1
    );
}

/// #2435: a model refused for the account says how to move on, once.
#[test]
fn a_model_refused_for_the_account_gets_the_refusal_guidance_once() {
    let refusal = r#"HTTP 400 from Codex: {"detail":"The 'mini' model is not supported when using Codex with a ChatGPT account."}"#;
    let out = explained(refusal, PLAIN);
    assert!(
        out.ends_with(
            "Model unavailable: the provider refused this model for the account or auth mode in \
             use. While a refusal is held (providers.model_refusal_ttl_secs, an hour by default; \
             0 holds none) the model is not offered, until the provider serves it again; choose \
             another model (list_models, then set_model)."
        ),
        "{out}"
    );
    assert_eq!(explained(&out, PLAIN), out);
}

#[test]
fn output_reads_from_whether_the_stream_emitted() {
    assert_eq!(Output::from_emitted(true), Output::Shown);
    assert_eq!(Output::from_emitted(false), Output::NotShown);
}

/// #2600 review: every class × facts branch of the approved guidance,
/// pinned exactly.
#[test]
fn every_retryable_class_and_fact_has_its_exact_guidance() {
    use Output::{NotShown, Shown};
    use Response::{NotReceived, Received, Unknown};
    let rate = "provider error (429): rate limit exceeded";
    let server = "HTTP 503 from OpenAI: service unavailable";
    let network = "HTTP error: error sending request: connection refused";
    let rows: [(&str, AttemptFacts, &str); 11] = [
        (
            rate,
            facts(2, Shown, Received),
            "Rate limit: the provider throttled the request partway through the reply. It was not retried, because a reply that has already streamed output is never replayed; continuing or re-prompting is safe once the limit clears.",
        ),
        (
            rate,
            facts(3, NotShown, Received),
            "Rate limit: the provider throttled the request. It was retried with backoff (3 attempts) and still failed — wait and retry later, or reduce request frequency.",
        ),
        (
            rate,
            facts(1, NotShown, Received),
            "Rate limit: the provider throttled the request. It was not retried — wait and retry later, or reduce request frequency.",
        ),
        (
            server,
            facts(1, Shown, Received),
            "Server/overload: the provider failed partway through the reply (overloaded or a 5xx error). It was not retried, because a reply that has already streamed output is never replayed; continuing or re-prompting is safe.",
        ),
        (
            server,
            facts(3, NotShown, Received),
            "Server/overload: the provider is overloaded or returned a 5xx error. It was retried (3 attempts) and still failed — retry later.",
        ),
        (
            server,
            facts(1, NotShown, Received),
            "Server/overload: the provider is overloaded or returned a 5xx error. It was not retried — retry later.",
        ),
        (
            network,
            facts(1, Shown, Received),
            "Network: the provider closed the connection partway through the reply. It was not retried, because a reply that has already streamed output is never replayed; continuing or re-prompting is safe.",
        ),
        (
            network,
            facts(3, NotShown, NotReceived),
            "Network: could not reach the provider (connection/timeout). It was retried (3 attempts) and still failed — check connectivity and retry later.",
        ),
        (
            network,
            facts(1, NotShown, NotReceived),
            "Network: could not reach the provider (connection/timeout). It was not retried — check connectivity and retry later.",
        ),
        (
            network,
            facts(2, NotShown, Unknown),
            "Network: the connection to the provider failed (connection/timeout). It was retried (2 attempts) and still failed — retry later.",
        ),
        (
            network,
            facts(1, NotShown, Received),
            "Network: the connection to the provider failed (connection/timeout). It was not retried — retry later.",
        ),
    ];
    for (message, at, expected) in rows {
        assert_eq!(guidance(message, at), expected, "{message} {at:?}");
    }
}

/// #2600 review: the classes whose guidance does not depend on attempts
/// keep their exact text, and an unknown failure passes through unchanged.
#[test]
fn auth_billing_and_unknown_failures_are_unchanged() {
    assert_eq!(
        guidance("provider error (401): invalid credentials", PLAIN),
        "Authentication: the provider rejected your credentials (not retryable). Check the API key or re-authenticate (`quecto auth login`), then retry."
    );
    assert_eq!(
        guidance(
            r#"HTTP 429 from OpenAI: {"error":{"code":"insufficient_quota"}}"#,
            PLAIN
        ),
        "Billing/quota: the provider reports exhausted credit or a billing limit (not retryable). Check the account plan, billing, or quota before retrying."
    );
    let unknown = "something unexpected went wrong";
    assert_eq!(
        explained(unknown, facts(3, Output::NotShown, Response::Received)),
        unknown
    );
}

/// #2600 review: the inference-admission gate's own errors say what to do
/// themselves; no provider guidance is appended, whatever their class.
#[test]
fn admission_gate_errors_get_no_provider_guidance() {
    for message in [
        "admission: admission transport failure: connection refused",
        "admission: admission authority connection closed",
        "admission: admission refused: capability revoked by an authority reset",
    ] {
        assert_eq!(
            explained(message, facts(2, Output::NotShown, Response::NotReceived)),
            message
        );
    }
}
