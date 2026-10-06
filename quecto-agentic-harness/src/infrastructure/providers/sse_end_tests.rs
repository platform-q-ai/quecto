use super::*;
use crate::domain::inference::services::provider_error::{
    ProviderErrorClass, classify_provider_error,
};

#[test]
fn done_is_recognised_with_trailing_whitespace_only() {
    for data in ["[DONE]", "[DONE] ", "[DONE]\t", "[DONE]\r"] {
        assert!(is_done_marker(data), "{data:?}");
    }
    for data in [" [DONE]", "[DONE]x", "[done]", "", "DONE"] {
        assert!(!is_done_marker(data), "{data:?}");
    }
}

/// Every vendor's cut-short error is one, classified as a retryable
/// network failure; the empty stream is one, retried as an empty stream.
#[test]
fn every_cut_short_error_is_recognised_and_retried() {
    for message in [
        super::super::codex::RESPONSES_CUT_SHORT,
        super::super::anthropic::anthropic_sse::ANTHROPIC_CUT_SHORT,
        super::super::openai::openai_sse::OPENAI_CUT_SHORT,
    ] {
        assert!(is_cut_short(message), "{message}");
        let class = classify_provider_error(&DomainError::Provider(message.into()));
        assert_eq!(class, ProviderErrorClass::Network, "{message}");
    }
    assert!(is_cut_short(EMPTY_STREAM));
    assert_eq!(
        classify_provider_error(&DomainError::Provider(EMPTY_STREAM.into())),
        ProviderErrorClass::EmptyStream
    );
    for message in ["stream read error: x", "HTTP 500 from Codex: boom", ""] {
        assert!(!is_cut_short(message), "{message}");
    }
    assert_eq!(
        ended_early(false, super::super::openai::openai_sse::OPENAI_CUT_SHORT),
        EMPTY_STREAM
    );
    assert_eq!(
        ended_early(true, super::super::openai::openai_sse::OPENAI_CUT_SHORT),
        super::super::openai::openai_sse::OPENAI_CUT_SHORT
    );
}

#[test]
fn a_last_line_is_the_bytes_after_the_last_newline() {
    assert!(last_line(b"").is_none());
    assert_eq!(last_line(b"data: [DONE]").unwrap().unwrap(), "data: [DONE]");
    assert_eq!(last_line(b"data: x\r").unwrap().unwrap(), "data: x");
    assert!(last_line(&[0xff, 0xfe]).unwrap().is_err());
}

#[test]
fn unfinished_usage_is_recorded_only_with_a_trace_and_usage() {
    let usage = UsageInfo {
        prompt_tokens: 7,
        completion_tokens: 0,
        cache_read_tokens: None,
        cache_write_tokens: None,
        context_tokens: None,
        cost: None,
    };
    let trace = RequestTrace::default();
    let reply = UnfinishedReply {
        error: DomainError::Provider("cut".into()),
        usage: Some(usage.clone()),
    };
    assert!(matches!(reply.account(Some(&trace), "m"), DomainError::Provider(m) if m == "cut"));
    assert_eq!(trace.unfinished_usage().len(), 1);
    assert_eq!(trace.unfinished_usage()[0].prompt_tokens, 7);
    record_unfinished_usage(None, Some(usage), "m");
    record_unfinished_usage(Some(&trace), None, "m");
    assert_eq!(trace.unfinished_usage().len(), 1);
}
