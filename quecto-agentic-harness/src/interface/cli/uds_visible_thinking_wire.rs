use crate::domain::message::ThinkingBlock;
use crate::domain::visible_thinking::VisibleThinkingPageBlock;

pub(super) fn visible_thinking_blocks_json(blocks: &[ThinkingBlock]) -> serde_json::Value {
    serde_json::Value::Array(blocks.iter().map(visible_thinking_block_json).collect())
}

fn visible_thinking_block_json(block: &ThinkingBlock) -> serde_json::Value {
    match block {
        ThinkingBlock::Normal { thinking, .. } => serde_json::json!({
            "kind": "text",
            "text": thinking,
        }),
        ThinkingBlock::Redacted { .. } => serde_json::json!({ "kind": "redacted" }),
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
