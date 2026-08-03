use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChildToolPolicyPropagation {
    #[serde(default)]
    pub agent_id: String,
    #[serde(default)]
    pub status: String,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

pub fn summarize_child_policy_propagation(
    child_propagation: &[ChildToolPolicyPropagation],
) -> (usize, usize) {
    child_propagation
        .iter()
        .fold((0, 0), |(ok, failed), item| match item.status.as_str() {
            "applied" | "queued" => (ok + 1, failed),
            _ => (ok, failed + 1),
        })
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ToolPolicyResult {
    #[serde(default)]
    pub after: Option<ToolCatalogueEntry>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolPolicyMutation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub scope: ToolScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ToolPolicyApplyMode {
    ImmediateIfIdle,
    AtNextTurnBoundary,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ToolScope {
    None,
    Parent,
    Child,
    Both,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolCatalogueEntry {
    #[serde(default)]
    pub stable_id: String,
    pub name: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub profile_scope: Option<ToolScope>,
    #[serde(default)]
    pub profile_enabled: Option<bool>,
    #[serde(default)]
    pub effective_scope: Option<ToolScope>,
    #[serde(default)]
    pub effective_parent_enabled: Option<bool>,
    #[serde(default)]
    pub effective_child_enabled: Option<bool>,
    #[serde(default)]
    pub effective_enabled: Option<bool>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}
