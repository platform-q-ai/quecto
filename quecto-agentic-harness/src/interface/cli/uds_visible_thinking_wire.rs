use crate::domain::conversation::services::visible_thinking::VisibleThinkingPageBlock;
use crate::domain::conversation::value_objects::message::ThinkingBlock;

pub(super) fn visible_thinking_blocks_json(blocks: &[ThinkingBlock]) -> serde_json::Value {
    serde_json::Value::Array(
        blocks
            .iter()
            .filter(|block| block.is_visible())
            .map(visible_thinking_block_json)
            .collect(),
    )
}

fn visible_thinking_block_json(block: &ThinkingBlock) -> serde_json::Value {
    match block {
        ThinkingBlock::Normal { thinking, .. } => serde_json::json!({
            "kind": "text",
            "text": thinking,
        }),
        ThinkingBlock::Redacted { .. } => serde_json::json!({ "kind": "redacted" }),
        // Filtered out above (#2162).
        ThinkingBlock::EncryptedReasoning { .. } => serde_json::json!({ "kind": "hidden" }),
    }
}

pub(super) fn visible_thinking_page_block_json(
    block: &VisibleThinkingPageBlock,
) -> serde_json::Value {
    match block {
        VisibleThinkingPageBlock::Text { text } => {
            serde_json::json!({ "kind": "text", "text": text })
        }
        VisibleThinkingPageBlock::Redacted => serde_json::json!({ "kind": "redacted" }),
    }
}

pub(super) fn visible_thinking_page_json(
    blocks: Vec<VisibleThinkingPageBlock>,
) -> serde_json::Value {
    serde_json::Value::Array(
        blocks
            .iter()
            .map(visible_thinking_page_block_json)
            .collect(),
    )
}

#[cfg(test)]
#[path = "uds_visible_thinking_wire_tests.rs"]
mod tests;
