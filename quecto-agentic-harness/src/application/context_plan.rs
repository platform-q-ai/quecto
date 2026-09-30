//! What one context-preparation pass did, and the tool-result message
//! input (split from `context.rs` for its decrease-only line ceiling, #2348).

use crate::domain::message::ToolCall;
use crate::domain::tool::ImageBlock;

#[derive(Debug, Clone, Default)]
pub(crate) struct ContextPlan {
    pub tokens_before: usize,
    pub total_tokens: usize,
    pub tool_results_collapsed: usize,
    pub messages_collapsed: usize,
    pub ladder_stubbed: usize,
    pub messages_dropped: usize,
    /// Tool results a newer snapshot of the same state superseded (#2342).
    pub snapshots_superseded: usize,
    /// Large tool results the size-aware rule collapsed (#2348).
    pub large_results_collapsed: usize,
    pub over_budget: bool,
    pub durable_prefix_dirty: bool,
}

pub(crate) struct ToolMessageBuild<'a> {
    pub tc: &'a ToolCall,
    pub content: String,
    pub image_blocks: Vec<ImageBlock>,
    pub is_error: bool,
}
