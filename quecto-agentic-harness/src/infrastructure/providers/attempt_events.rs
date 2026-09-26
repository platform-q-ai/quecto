//! The stream events attempt diagnostics know (#2158): an event outside this
//! list counts as unknown, so a vendor's new or renamed event shows up in
//! the diagnostics instead of hiding among normal traffic.

/// Every non-terminal event type the providers' stream handlers consume, or
/// that a vendor sends as ordinary traffic. Terminal events are classified
/// separately.
pub(super) const KNOWN_EVENTS: &[&str] = &[
    // Responses API (Codex).
    "response.created",
    "response.in_progress",
    "response.output_item.added",
    "response.output_item.done",
    "response.content_part.added",
    "response.content_part.done",
    "response.output_text.delta",
    "response.output_text.done",
    "response.function_call_arguments.delta",
    "response.function_call_arguments.done",
    "response.refusal.delta",
    "response.refusal.done",
    "response.reasoning_summary_part.added",
    "response.reasoning_summary_part.done",
    "response.reasoning_summary_text.delta",
    "response.reasoning_summary_text.done",
    "response.reasoning.summary_text.delta",
    // Anthropic Messages.
    "message_start",
    "message_delta",
    "content_block_start",
    "content_block_delta",
    "content_block_stop",
    "ping",
];

#[cfg(test)]
#[path = "attempt_events_tests.rs"]
mod tests;
