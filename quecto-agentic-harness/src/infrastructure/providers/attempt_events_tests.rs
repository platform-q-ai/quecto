use super::KNOWN_EVENTS;

/// Every event type the Codex stream handler matches on is known, so the
/// diagnostics never count the handler's own traffic as unknown (#2158).
#[test]
fn every_event_the_codex_handler_consumes_is_known() {
    let terminal = [
        "response.completed",
        "response.failed",
        "response.incomplete",
    ];
    for source in [
        include_str!("codex_sse_state.rs"),
        include_str!("codex_sse_handler.rs"),
    ] {
        for event in source
            .split('"')
            .skip(1)
            .step_by(2)
            .filter(|literal| literal.starts_with("response."))
        {
            assert!(
                KNOWN_EVENTS.contains(&event) || terminal.contains(&event),
                "{event} is consumed by the Codex handler but not known to diagnostics"
            );
        }
    }
}
