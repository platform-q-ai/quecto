//! #2410: a Messages request measured block by block, cache markers left
//! out, for the input-prefix diagnostic.
use super::*;
use serde_json::json;

fn body(messages: serde_json::Value) -> Value {
    json!({
        "model": "claude-sonnet-4-5",
        "system": [{"type": "text", "text": "sys", "cache_control": {"type": "ephemeral"}}],
        "tools": [{"name": "bash", "description": "run", "input_schema": {}}],
        "messages": messages,
    })
}

fn compare(
    before: &Value,
    after: &Value,
) -> crate::domain::inference::events::request_observation::InputPrefixParts {
    use crate::domain::inference::events::request_observation::InputBaseline;
    use crate::infrastructure::providers::transport::input_prefix::{commit, compare};
    let baseline = InputBaseline::default();
    commit(&baseline, measured("s", "e", before));
    compare(&baseline, &measured("s", "e", after))
        .expect("a comparison")
        .parts()
}

/// Each content block of each message is one item, a string content one
/// text block.
#[test]
fn every_content_block_is_an_item_of_its_kind() {
    let six = body(json!([
        {"role": "user", "content": "go"},
        {"role": "assistant", "content": [
            {"type": "thinking", "thinking": "hmm", "signature": "sig"},
            {"type": "text", "text": "ok"},
            {"type": "tool_use", "id": "t1", "name": "bash", "input": {}},
        ]},
        {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "t1", "content": "done"},
            {"type": "image", "source": {"type": "base64", "media_type": "image/png", "data": "x"}},
        ]},
    ]));
    assert_eq!(compare(&body(json!([])), &six).input_items, 6);
    use crate::domain::inference::events::request_observation::InputItemKind::*;
    for (role, kind_name, expected) in [
        ("user", "text", Some(User)),
        ("user", "image", Some(User)),
        ("user", "tool_result", Some(FunctionCallOutput)),
        ("assistant", "text", Some(Assistant)),
        ("assistant", "tool_use", Some(FunctionCall)),
        ("assistant", "thinking", Some(Reasoning)),
        ("assistant", "redacted_thinking", Some(Reasoning)),
        ("user", "document", None),
        ("assistant", "tool_result", None),
    ] {
        assert_eq!(
            super::kind(role, &json!({"type": kind_name})),
            expected,
            "{role} {kind_name}"
        );
    }
}

/// #2410: the cache markers move forward every request; a block that lost
/// or gained one, or a string content sent as its one marked text block,
/// reads as unchanged.
#[test]
fn a_moved_cache_marker_is_no_edit() {
    let before = body(json!([
        {"role": "user", "content": [{"type": "text", "text": "go", "cache_control": {"type": "ephemeral"}}]},
    ]));
    let after = body(json!([
        {"role": "user", "content": "go"},
        {"role": "assistant", "content": [{"type": "text", "text": "ok"}]},
        {"role": "user", "content": [{"type": "text", "text": "next", "cache_control": {"type": "ephemeral"}}]},
    ]));
    let prefix = compare(&before, &after);
    assert_eq!(prefix.previous_items, Some(1));
    assert_eq!(prefix.first_changed_item, None, "append-only: {prefix:?}");
    assert!(prefix.unchanged_prefix_tokens_estimate > 0);
}

/// An edited block records where the inputs diverge, and its kind.
#[test]
fn an_edited_tool_result_records_its_index_and_kind() {
    let turn = |output: &str| {
        body(json!([
            {"role": "user", "content": "go"},
            {"role": "assistant", "content": [{"type": "tool_use", "id": "t1", "name": "bash", "input": {}}]},
            {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t1", "content": output}]},
        ]))
    };
    let prefix = compare(&turn("a long output"), &turn("[pruned]"));
    assert_eq!(prefix.first_changed_item, Some(2));
    assert_eq!(
        prefix.first_changed_kind,
        Some(crate::domain::inference::events::request_observation::InputItemKind::FunctionCallOutput)
    );
}

/// A changed system prompt leaves nothing of the head cached.
#[test]
fn a_changed_system_prompt_caches_nothing_before_the_conversation() {
    let messages = json!([{"role": "user", "content": "go"}]);
    let before = body(messages.clone());
    let mut after = body(messages);
    after["system"] = json!([{"type": "text", "text": "another"}]);
    assert_eq!(compare(&before, &after).unchanged_prefix_tokens_estimate, 0);
}

/// An image is estimated from its pixel size, not its base64 text.
#[test]
fn an_unreadable_image_costs_the_most_an_image_can() {
    let block = json!({"type": "image", "source": {"type": "url", "url": "https://x"}});
    assert_eq!(super::image_tokens(&block), UNREADABLE_IMAGE_TOKENS);
}

/// #2628 review: what the request asks the model to do with its thinking
/// and its output selects the cache as the system prompt does: a change in
/// either leaves nothing of the head cached.
#[test]
fn a_changed_thinking_or_output_config_caches_nothing_before_the_conversation() {
    let messages = json!([{"role": "user", "content": "go"}]);
    for (field, before, after) in [
        (
            "thinking",
            json!({"type": "enabled", "budget_tokens": 1024}),
            json!({"type": "enabled", "budget_tokens": 2048}),
        ),
        (
            "output_config",
            json!({"effort": "low"}),
            json!({"effort": "high"}),
        ),
    ] {
        let mut first = body(messages.clone());
        first[field] = before;
        let mut second = body(messages.clone());
        second[field] = after;
        assert_eq!(
            compare(&first, &second).unchanged_prefix_tokens_estimate,
            0,
            "{field}"
        );
        assert!(
            compare(&first, &first).unchanged_prefix_tokens_estimate > 0,
            "{field}"
        );
    }
}
