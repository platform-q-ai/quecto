//! Extension system — TUI-registered tools and execute_tool handling.
//!
//! The TUI can register tools with the quecto agent via `register_tools`,
//! and handle `execute_tool` events by returning `tool_result` responses.

use crate::client::{Command, CommandSender};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A tool that the TUI provides to the agent.
#[derive(Debug, Clone)]
pub struct TuiTool {
    pub name: String,
    pub description: String,
    pub parameters_schema: String,
}

impl TuiTool {
    pub fn new(name: &str, description: &str) -> Self {
        Self {
            name: name.to_string(),
            description: description.to_string(),
            parameters_schema: r#"{"type":"object"}"#.to_string(),
        }
    }

    pub fn with_schema(mut self, schema: &str) -> Self {
        self.parameters_schema = schema.to_string();
        self
    }
}

/// Pending tool execution waiting for a result.
#[derive(Debug)]
pub struct PendingExecution {
    pub tool_call_id: String,
    pub tool_name: String,
    pub arguments: String,
}

/// Manages TUI-provided tools and pending executions.
pub struct ExtensionManager {
    tools: Vec<TuiTool>,
    pending: HashMap<String, PendingExecution>,
}

impl ExtensionManager {
    pub fn new() -> Self {
        Self {
            tools: Vec::new(),
            pending: HashMap::new(),
        }
    }

    /// Register a tool that the TUI provides.
    pub fn register_tool(&mut self, tool: TuiTool) {
        // Replace if already registered.
        self.tools.retain(|t| t.name != tool.name);
        self.tools.push(tool);
    }

    /// Build the `register_tools` command to send to the agent.
    pub fn build_register_command(&self) -> serde_json::Value {
        let tools: Vec<serde_json::Value> = self
            .tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "parametersSchema": t.parameters_schema,
                })
            })
            .collect();

        serde_json::json!({
            "type": "register_tools",
            "tools": tools,
        })
    }

    /// Record an incoming execute_tool event as pending.
    pub fn on_execute_tool(&mut self, tool_call_id: &str, tool_name: &str, arguments: &str) {
        self.pending.insert(
            tool_call_id.to_string(),
            PendingExecution {
                tool_call_id: tool_call_id.to_string(),
                tool_name: tool_name.to_string(),
                arguments: arguments.to_string(),
            },
        );
    }

    /// Take a pending execution by call ID (removes it from pending).
    pub fn take_pending(&mut self, tool_call_id: &str) -> Option<PendingExecution> {
        self.pending.remove(tool_call_id)
    }

    /// Check if a tool name is registered.
    pub fn has_tool(&self, name: &str) -> bool {
        self.tools.iter().any(|t| t.name == name)
    }

    /// Number of pending executions.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// Tool names registered.
    pub fn tool_names(&self) -> Vec<&str> {
        self.tools.iter().map(|t| t.name.as_str()).collect()
    }
}

impl Default for ExtensionManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a `tool_result` command to send back to the agent.
pub fn build_tool_result(tool_call_id: &str, content: &str, is_error: bool) -> serde_json::Value {
    serde_json::json!({
        "type": "tool_result",
        "toolCallId": tool_call_id,
        "content": content,
        "isError": is_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_tool() {
        let mut mgr = ExtensionManager::new();
        mgr.register_tool(TuiTool::new("tui_confirm", "Show confirm dialog"));
        assert!(mgr.has_tool("tui_confirm"));
        assert_eq!(mgr.tool_names(), vec!["tui_confirm"]);
    }

    #[test]
    fn register_replaces_existing() {
        let mut mgr = ExtensionManager::new();
        mgr.register_tool(TuiTool::new("test", "v1"));
        mgr.register_tool(TuiTool::new("test", "v2"));
        assert_eq!(mgr.tool_names().len(), 1);
    }

    #[test]
    fn build_register_command() {
        let mut mgr = ExtensionManager::new();
        mgr.register_tool(TuiTool::new("tui_confirm", "Show confirm"));
        let cmd = mgr.build_register_command();
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("register_tools"));
        assert!(json.contains("tui_confirm"));
    }

    #[test]
    fn pending_execution() {
        let mut mgr = ExtensionManager::new();
        mgr.on_execute_tool("tc-1", "tui_confirm", r#"{"message":"ok?"}"#);
        assert_eq!(mgr.pending_count(), 1);
        let exec = mgr.take_pending("tc-1").unwrap();
        assert_eq!(exec.tool_name, "tui_confirm");
        assert_eq!(mgr.pending_count(), 0);
    }

    #[test]
    fn take_pending_unknown_returns_none() {
        let mut mgr = ExtensionManager::new();
        assert!(mgr.take_pending("nonexistent").is_none());
    }

    #[test]
    fn tool_result_json() {
        let result = build_tool_result("tc-1", "confirmed", false);
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("tool_result"));
        assert!(json.contains("tc-1"));
        assert!(json.contains("confirmed"));
    }

    #[test]
    fn tool_result_error() {
        let result = build_tool_result("tc-2", "failed", true);
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"isError\":true"));
    }

    #[test]
    fn custom_schema() {
        let tool = TuiTool::new("test", "desc")
            .with_schema(r#"{"type":"object","properties":{"msg":{"type":"string"}}}"#);
        assert!(tool.parameters_schema.contains("msg"));
    }
}
