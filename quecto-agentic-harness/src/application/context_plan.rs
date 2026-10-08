//! What one context-preparation pass did, and the tool-result message
//! input (split from `context.rs` for its decrease-only line ceiling, #2348).

use crate::domain::conversation::value_objects::message::ToolCall;
use crate::domain::tool_policy::value_objects::tool::ImageBlock;

#[derive(Debug, Clone, Default)]
pub(crate) struct ContextPlan {
    pub tokens_before: usize,
    pub total_tokens: usize,
    /// What the emergency ladder stubbed, then dropped (#2414).
    pub ladder_stubbed: usize,
    pub messages_dropped: usize,
    pub over_budget: bool,
    /// Calls a cut or the ladder removed with their results, moved out (#2348).
    pub dropped_calls: Vec<ToolCall>,
    pub durable_prefix_dirty: bool,
}

pub(crate) struct ToolMessageBuild<'a> {
    pub tc: &'a ToolCall,
    pub content: String,
    pub image_blocks: Vec<ImageBlock>,
    pub is_error: bool,
}
