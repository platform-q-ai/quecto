//! The output one stream event carries (#2210): the bytes of text,
//! thinking, refusal and tool-call argument deltas, never the JSON around
//! them, so the count tracks what the model generated whatever each
//! vendor's event framing costs. It feeds the attempt's `output_bytes`,
//! `get_state`'s live progress and the output cap.
use super::super::attempt_profile::Vendor;
use serde_json::Value;

/// Responses API (Codex) events whose `delta` string is generated output.
/// Each is also a known event (#2158).
const RESPONSES_OUTPUT_DELTAS: &[&str] = &[
    "response.output_text.delta",
    "response.function_call_arguments.delta",
    "response.refusal.delta",
    "response.reasoning_summary_text.delta",
    "response.reasoning.summary_text.delta",
];

/// OpenAI chat `choices[].delta` fields that hold generated output. The
/// reasoning is one of [`CHAT_REASONING_FIELDS`], counted apart.
const CHAT_DELTA_FIELDS: &[&str] = &["content", "refusal"];

/// The names OpenAI-compatible providers stream reasoning under: one or the
/// other, never both. As the handler reads it (`openai_sse.rs`), the first
/// present is the reasoning (#2210 review).
const CHAT_REASONING_FIELDS: [&str; 2] = ["reasoning", "reasoning_content"];

/// Anthropic `content_block_delta` fields that hold generated output:
/// `text_delta`, `thinking_delta` and `input_json_delta`.
const MESSAGES_DELTA_FIELDS: &[&str] = &["text", "thinking", "partial_json"];

/// The bytes of output the event `value` of `vendor` carries.
pub(in crate::infrastructure::providers) fn output_bytes(vendor: Vendor, value: &Value) -> u64 {
    match vendor {
        Vendor::Codex => responses(value),
        Vendor::OpenAi => chat(value),
        Vendor::Anthropic => messages(value),
    }
}

fn text_len(value: &Value) -> u64 {
    value.as_str().map_or(0, |text| text.len() as u64)
}

fn responses(value: &Value) -> u64 {
    match value["type"].as_str() {
        Some(kind) if RESPONSES_OUTPUT_DELTAS.contains(&kind) => text_len(&value["delta"]),
        _ => 0,
    }
}

fn chat(value: &Value) -> u64 {
    let Some(choices) = value["choices"].as_array() else {
        return 0;
    };
    choices
        .iter()
        .map(|choice| {
            let delta = &choice["delta"];
            let [reasoning, alternative] = CHAT_REASONING_FIELDS;
            let reasoning = delta
                .get(reasoning)
                .or_else(|| delta.get(alternative))
                .map_or(0, text_len);
            let fields: u64 = CHAT_DELTA_FIELDS
                .iter()
                .map(|field| text_len(&delta[*field]))
                .sum::<u64>()
                .saturating_add(reasoning);
            let arguments: u64 = delta["tool_calls"].as_array().map_or(0, |calls| {
                calls
                    .iter()
                    .map(|call| text_len(&call["function"]["arguments"]))
                    .sum()
            });
            fields.saturating_add(arguments)
        })
        .fold(0, u64::saturating_add)
}

fn messages(value: &Value) -> u64 {
    MESSAGES_DELTA_FIELDS
        .iter()
        .map(|field| text_len(&value["delta"][*field]))
        .sum()
}

#[cfg(test)]
#[path = "attempt_output_tests.rs"]
mod tests;
