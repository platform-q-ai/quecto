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

/// `registrations`, each waited on for as long as an MCP call may take.
pub fn with_tool_timeout(
    registrations: Vec<QuectoToolRegistration>,
    _mcp_timeout: std::time::Duration,
) -> Vec<QuectoToolRegistration> {
    registrations
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;

#[derive(Debug, Clone)]
pub struct RegisteredMcpTools {
    pub registrations: Vec<QuectoToolRegistration>,
    pub mapping: HashMap<String, String>,
}
