//! Decode a `result` line (#2285): the turn-end essentials strictly, every
//! other field on its own with a default. A result is always a turn end; one
//! whose essentials are malformed is a failed one.

use serde_json::Value;

use super::json_fields::{Object, Strict, count, count_of, list, number, object, text, texts};
use crate::domain::external_agent::stream::{
    ModelUsage, PermissionDenial, ResultEvent, TokenCounts,
};

pub(super) fn result(line: &Object) -> ResultEvent {
    let is_error = Strict::of(line.get("is_error"), Value::as_bool);
    let terminal_reason = Strict::of(line.get("terminal_reason"), |v| {
        v.as_str().map(str::to_string)
    });
    let result_text = Strict::of(line.get("result"), |v| v.as_str().map(str::to_string));
    let total_cost_usd = Strict::of(line.get("total_cost_usd"), Value::as_f64);
    let mut errors = texts(line, "errors");
    let malformed: Vec<&str> = [
        ("is_error", is_error == Strict::Malformed),
        ("terminal_reason", terminal_reason == Strict::Malformed),
        ("result", result_text == Strict::Malformed),
        ("total_cost_usd", total_cost_usd == Strict::Malformed),
    ]
    .into_iter()
    .filter_map(|(field, bad)| bad.then_some(field))
    .collect();
    errors.extend(
        malformed
            .iter()
            .map(|field| format!("malformed result field `{field}`")),
    );
    ResultEvent {
        // A malformed essential fails the turn, whatever the rest says.
        is_error: match malformed.as_slice() {
            [] => is_error.present(),
            [_, ..] => Some(true),
        },
        terminal_reason: terminal_reason.present(),
        stop_reason: text(line, "stop_reason"),
        api_error_status: line.get("api_error_status").and_then(http_status),
        result_text: result_text.present(),
        errors,
        usage: object(line, "usage").map_or_else(TokenCounts::default, |usage| {
            tokens(
                usage,
                "input_tokens",
                "output_tokens",
                "cache_read_input_tokens",
                "cache_creation_input_tokens",
            )
        }),
        total_cost_usd: total_cost_usd.present(),
        model_usage: object(line, "modelUsage")
            .map(|models| {
                models
                    .iter()
                    .filter_map(|(model, usage)| model_usage(model, usage.as_object()?))
                    .collect()
            })
            .unwrap_or_default(),
        permission_denials: list(line, "permission_denials")
            .iter()
            .filter_map(Value::as_object)
            .map(|denial| PermissionDenial {
                tool_name: text(denial, "tool_name").unwrap_or_default(),
                tool_use_id: text(denial, "tool_use_id").unwrap_or_default(),
                tool_input: denial.get("tool_input").cloned().unwrap_or(Value::Null),
            })
            .collect(),
        num_turns: count(line, "num_turns").and_then(|n| u32::try_from(n).ok()),
        duration_ms: count(line, "duration_ms"),
        duration_api_ms: count(line, "duration_api_ms"),
        user_turn_ids: user_turn_ids(line),
    }
}

/// The user turns a result names (#2287): `user_message_uuids` (every one
/// the turn consumed), else `user_message_uuid`; an id that is not a
/// non-empty string is dropped.
fn user_turn_ids(line: &Object) -> Vec<String> {
    let named = |value: &Value| {
        value
            .as_str()
            .filter(|id| !id.is_empty())
            .map(str::to_string)
    };
    let all: Vec<String> = list(line, "user_message_uuids")
        .iter()
        .filter_map(named)
        .collect();
    match all.as_slice() {
        [] => line
            .get("user_message_uuid")
            .and_then(named)
            .into_iter()
            .collect(),
        [_, ..] => all,
    }
}

fn model_usage(model: &str, usage: &Object) -> Option<ModelUsage> {
    Some(ModelUsage {
        model: model.to_string(),
        tokens: tokens(
            usage,
            "inputTokens",
            "outputTokens",
            "cacheReadInputTokens",
            "cacheCreationInputTokens",
        ),
        cost_usd: number(usage, "costUSD"),
    })
}

/// Token counts; an absent or malformed count is zero.
fn tokens(
    usage: &Object,
    input: &str,
    output: &str,
    cache_read: &str,
    cache_write: &str,
) -> TokenCounts {
    let read = |key: &str| usage.get(key).and_then(count_of).unwrap_or(0);
    TokenCounts {
        input: read(input),
        output: read(output),
        cache_read: read(cache_read),
        cache_write: read(cache_write),
    }
}

/// An `api_error_status` as an HTTP status: a number, or a numeric string.
fn http_status(value: &Value) -> Option<u16> {
    match value {
        Value::Number(number) => number.as_u64().and_then(|n| u16::try_from(n).ok()),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}
