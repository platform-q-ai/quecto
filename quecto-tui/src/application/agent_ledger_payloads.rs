//! Typed application values for direct sub-agent ledger synchronization payloads.
//!
//! The infrastructure client receives raw JSON from UDS, but the agents
//! presentation policy stores typed ledger messages and capability snapshots.

use serde::{Deserialize, Deserializer};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncDelta {
    pub epoch: u64,
    pub rev: u64,
    #[serde(default)]
    pub messages: Vec<LedgerMessage>,
    pub next_rev: Option<u64>,
    #[serde(default)]
    pub caught_up: bool,
    #[serde(default)]
    pub resync: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerMessage {
    pub id: Option<String>,
    pub role: Option<String>,
    pub content: Option<String>,
    #[serde(default, alias = "tool_calls")]
    pub tool_calls: Vec<LedgerToolCall>,
    #[serde(default, alias = "tool_call_id")]
    pub tool_call_id: Option<String>,
    #[serde(default, alias = "tool_name")]
    pub tool_name: Option<String>,
    #[serde(default, alias = "is_error")]
    pub is_error: bool,
}

impl LedgerMessage {
    pub fn role(&self) -> &str {
        self.role.as_deref().unwrap_or("")
    }

    pub fn content(&self) -> &str {
        self.content.as_deref().unwrap_or("")
    }

    pub fn tool_call_id(&self) -> &str {
        self.tool_call_id.as_deref().unwrap_or("")
    }

    pub fn tool_name(&self) -> &str {
        self.tool_name.as_deref().unwrap_or("tool")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedToolArgs(serde_json::Value);

impl ParsedToolArgs {
    pub fn into_value(self) -> serde_json::Value {
        self.0
    }
}

pub fn parse_tool_args(args: &str) -> Option<ParsedToolArgs> {
    serde_json::from_str(args).ok().map(ParsedToolArgs)
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LedgerToolCall {
    id: Option<String>,
    name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_json_string")]
    arguments: Option<String>,
    function: Option<LedgerFunctionCall>,
}

impl LedgerToolCall {
    pub fn id(&self) -> &str {
        self.id.as_deref().unwrap_or("")
    }

    pub fn name(&self) -> &str {
        self.name
            .as_deref()
            .or_else(|| self.function.as_ref().and_then(|f| f.name.as_deref()))
            .unwrap_or("tool")
    }

    pub fn arguments(&self) -> String {
        self.arguments
            .clone()
            .or_else(|| self.function.as_ref().and_then(|f| f.arguments.clone()))
            .unwrap_or_else(|| "{}".to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LedgerFunctionCall {
    name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_json_string")]
    arguments: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncCapability {
    pub sync: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_capability_set")]
    capabilities: Option<CapabilitySet>,
}

impl SyncCapability {
    pub fn supports_sync(&self) -> bool {
        self.sync.unwrap_or(0) >= 1
            || self
                .capabilities
                .as_ref()
                .unwrap_or(&CapabilitySet::NONE)
                .sync
                .unwrap_or(0)
                >= 1
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CapabilitySet {
    sync: Option<u64>,
}

impl CapabilitySet {
    const NONE: Self = Self { sync: None };
}

pub fn supports_sync(value: &serde_json::Value) -> bool {
    serde_json::from_value::<SyncCapability>(value.clone())
        .is_ok_and(|capability| capability.supports_sync())
}

fn deserialize_capability_set<'de, D>(deserializer: D) -> Result<Option<CapabilitySet>, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<CapabilitySet>::deserialize(deserializer).unwrap_or(None))
}

fn deserialize_optional_json_string<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    Ok(value.map(|value| {
        value
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| value.to_string())
    }))
}
