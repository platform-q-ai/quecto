//! Result helpers for the swarm tool (split from `swarm.rs` for the 750-line
//! cap).
use crate::domain::error::DomainError;
use crate::domain::tool::ToolResult;

pub(super) fn tool_err(content: String) -> Result<ToolResult, DomainError> {
    Ok(ToolResult {
        content,
        is_error: true,
        image_blocks: vec![],
        delivery_metadata: None,
    })
}
pub(super) fn ok_json(v: serde_json::Value, is_error: bool) -> Result<ToolResult, DomainError> {
    Ok(ToolResult {
        content: serde_json::to_string_pretty(&v).unwrap(),
        is_error,
        image_blocks: vec![],
        delivery_metadata: None,
    })
}
