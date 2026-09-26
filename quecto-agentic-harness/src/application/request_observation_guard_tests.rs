use super::*;

/// #2151: a published observation carries the request's time to first
/// token, taken from its attempts.
#[test]
fn a_published_observation_carries_the_time_to_first_token() {
    let log = Mutex::new(RequestDiagnostics::default());
    let messages = vec![crate::domain::message::Message::user("hi")];
    let request = ChatRequest {
        trace: None,
        admission: None,
        model: "m",
        messages: &messages,
        tools: &[],
        max_tokens: 1,
        temperature: 0.0,
        thinking_level: None,
        effort: None,
        tool_choice: None,
        metadata: None,
        session_id: None,
        cancel_flag: None,
    };
    let trace = Arc::new(RequestTrace::default());
    let mut guard = ObservationGuard::new(
        ObservationSinks {
            log: &log,
            outbox: None,
        },
        &request,
        "p",
        0,
        PrefixObservation {
            sha256: String::new(),
            bytes: 0,
            unchanged: None,
        },
        trace.clone(),
    );
    std::thread::sleep(std::time::Duration::from_millis(20));
    trace.mark_first_token(std::time::Instant::now());
    let record = guard.finish(&Err(DomainError::Other("stopped".into())));
    let first = record.first_token_ms.expect("time to first token");
    assert!(first >= 20, "{first}");
    assert!(
        first <= record.duration_ms,
        "{first} {}",
        record.duration_ms
    );
}
