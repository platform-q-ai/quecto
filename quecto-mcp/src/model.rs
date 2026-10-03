use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuectoToolRegistration {
    pub name: String,
    pub description: String,
    #[serde(rename = "parametersSchema")]
    pub parameters_schema: String,
    /// How long Quecto waits for this tool's result, in whole seconds
    /// (#2423); absent is Quecto's default (30).
    #[serde(
        rename = "timeoutSeconds",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub timeout_seconds: Option<u64>,
}

/// How much longer than an MCP call's HTTP timeout Quecto waits for its
/// result, so the bridge's own timeout error, which names the MCP call,
/// reaches the model rather than Quecto's.
pub const TIMEOUT_MARGIN_SECONDS: u64 = 5;

/// The timeouts Quecto allows a tool, in whole seconds (#2423).
pub const QUECTO_TIMEOUT_SECONDS: std::ops::RangeInclusive<u64> = 1..=600;

/// `registrations`, each waited on as long as an MCP call may take: the
/// MCP HTTP timeout (`--timeout`, rounded up to whole seconds) plus
/// [`TIMEOUT_MARGIN_SECONDS`], within [`QUECTO_TIMEOUT_SECONDS`].
pub fn with_tool_timeout(
    registrations: Vec<QuectoToolRegistration>,
    mcp_timeout: std::time::Duration,
) -> Vec<QuectoToolRegistration> {
    let whole = mcp_timeout
        .as_secs()
        .saturating_add(u64::from(mcp_timeout.subsec_nanos() > 0));
    let seconds = whole.saturating_add(TIMEOUT_MARGIN_SECONDS).clamp(
        *QUECTO_TIMEOUT_SECONDS.start(),
        *QUECTO_TIMEOUT_SECONDS.end(),
    );
    assert!(QUECTO_TIMEOUT_SECONDS.contains(&seconds));
    registrations
        .into_iter()
        .map(|registration| QuectoToolRegistration {
            timeout_seconds: Some(seconds),
            ..registration
        })
        .collect()
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;

#[derive(Debug, Clone)]
pub struct RegisteredMcpTools {
    pub registrations: Vec<QuectoToolRegistration>,
    pub mapping: HashMap<String, String>,
}
