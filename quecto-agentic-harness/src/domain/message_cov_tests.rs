use super::*;

#[test]
fn message_constructors_cover_user_assistant_and_tool_roles() {
    let user = Message::user("hello");
    assert_eq!(user.role, Role::User);
    assert_eq!(user.content, "hello");

    let tool_call = ToolCall {
        id: "call-1".to_string(),
        name: "bash".to_string(),
        arguments: "{}".to_string(),
    };
    let assistant = Message::assistant("using tool", vec![tool_call]);
    assert_eq!(assistant.role, Role::Assistant);
    assert_eq!(assistant.tool_calls.len(), 1);
    assert_eq!(assistant.tool_calls[0].id, "call-1");

    let tool = Message::tool("call-1".to_string(), "done".to_string());
    assert_eq!(tool.role, Role::Tool);
    assert_eq!(tool.tool_call_id.as_deref(), Some("call-1"));
    assert_eq!(tool.content, "done");
}

#[test]
fn cost_info_total_cost_usd_uses_total_micro_usd() {
    let cost = CostInfo {
        input_cost_micro_usd: 100_000,
        output_cost_micro_usd: 200_000,
        cache_read_cost_micro_usd: 30_000,
        cache_write_cost_micro_usd: 40_000,
        total_cost_micro_usd: 370_000,
    };

    assert_eq!(cost.total_cost_usd(), 0.37);
}

#[test]
fn estimated_tokens_counts_content_tool_calls_ids_and_images_once() {
    let mut msg = Message::assistant(
        "abcdé",
        vec![ToolCall {
            id: "tc1".to_string(),
            name: "tool".to_string(),
            arguments: "abcdefghi".to_string(),
        }],
    );
    msg.tool_call_id = Some("callid".to_string());
    msg.image_blocks.push(crate::domain::tool::ImageBlock {
        mime_type: "image/png",
        data: "12345".to_string(),
    });
    msg.user_image_blocks.push(UserImageBlock {
        mime_type: "image/jpeg".to_string(),
        data: "abcdef".to_string(),
    });

    let expected = Message::estimate_tokens("abcdé")
        + Message::estimate_tokens("tool")
        + Message::estimate_tokens("abcdefghi")
        + Message::estimate_tokens("callid")
        + Message::estimate_tokens("12345")
        + Message::estimate_tokens("abcdef");

    assert_eq!(msg.estimated_tokens(), expected);
    assert_eq!(msg.estimated_tokens(), expected);
    assert_eq!(msg.cached_token_build_count_for_tests(), 1);
}

#[test]
fn cloned_message_gets_a_fresh_token_cache() {
    let msg = Message::user("abcdefgh");
    assert_eq!(msg.estimated_tokens(), 2);
    assert_eq!(msg.cached_token_build_count_for_tests(), 1);

    let cloned = msg.clone();
    assert_eq!(cloned.cached_token_build_count_for_tests(), 0);
    assert_eq!(cloned.estimated_tokens(), 2);
    assert_eq!(cloned.cached_token_build_count_for_tests(), 1);
    assert_eq!(msg.cached_token_build_count_for_tests(), 1);
}

#[test]
fn token_cache_clone_directly_resets_once_lock() {
    let cache = TokenCache::default();
    let cloned = cache.clone();
    assert!(cloned.tokens.get().is_none());
    assert_eq!(
        cloned.build_count.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
}
