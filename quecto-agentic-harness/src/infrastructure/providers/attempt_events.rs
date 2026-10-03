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

/// The terminal events of the Responses and Messages wires, named in an
/// attempt's event counts as the known events are.
const TERMINAL_EVENTS: &[&str] = &[
    "response.completed",
    "response.failed",
    "response.incomplete",
    "error",
    "message_stop",
];

/// The type an attempt's event counts file an event under (#2433): its
/// name when the harness knows it — for the Messages wire the `event:` name
/// before it, `anthropic_event` — and `(unknown)` otherwise, so a record
/// never holds a name a provider made up. An OpenAI chat chunk has no
/// type: it is named by what its first choice carries.
pub(super) fn event_kind(
    vendor: super::super::attempt_profile::Vendor,
    anthropic_event: &str,
    value: &serde_json::Value,
) -> &'static str {
    use super::super::attempt_profile::Vendor;
    let named = match vendor {
        Vendor::Anthropic => anthropic_event,
        Vendor::Codex => value["type"].as_str().unwrap_or_default(),
        Vendor::OpenAi => return chat_kind(value),
    };
    KNOWN_EVENTS
        .iter()
        .chain(TERMINAL_EVENTS)
        .find(|known| **known == named)
        .copied()
        .unwrap_or(UNKNOWN_KIND)
}

/// The type of an event the harness does not know.
pub(super) const UNKNOWN_KIND: &str = "(unknown)";

/// An OpenAI chat chunk's type: the first delta field of its first choice
/// that carries something, else its finish, else `chat.empty`.
fn chat_kind(value: &serde_json::Value) -> &'static str {
    let choice = &value["choices"][0];
    let delta = &choice["delta"];
    let carries = |field: &str| match &delta[field] {
        serde_json::Value::String(text) => !text.is_empty(),
        serde_json::Value::Array(items) => !items.is_empty(),
        _ => false,
    };
    [
        ("tool_calls", "chat.delta.tool_calls"),
        ("content", "chat.delta.content"),
        ("reasoning", "chat.delta.reasoning"),
        ("reasoning_content", "chat.delta.reasoning"),
        ("refusal", "chat.delta.refusal"),
    ]
    .into_iter()
    .find(|(field, _)| carries(field))
    .map(|(_, kind)| kind)
    .unwrap_or(match choice["finish_reason"].is_string() {
        true => "chat.finish",
        false => "chat.empty",
    })
}

#[cfg(test)]
#[path = "attempt_events_tests.rs"]
mod tests;
