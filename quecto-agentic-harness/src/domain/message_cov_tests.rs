use super::*;

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
