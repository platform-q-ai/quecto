//! Typed application values for direct sub-agent ledger synchronization payloads.
//!
//! The infrastructure client receives raw JSON from UDS, but the agents
//! presentation policy stores typed ledger messages and capability snapshots.

use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncDelta {
    pub epoch: u64,
    pub rev: u64,
    // Messages decode leniently, per field and per message. The pre-typed
    // implementation stored raw JSON and defaulted malformed fields at
    // projection time, so a single malformed message must never discard an
    // entire sync delta (which would silently stall the revision cursor).
    #[serde(default)]
    pub messages: Vec<LedgerMessage>,
    pub next_rev: Option<u64>,
    #[serde(default)]
    pub caught_up: bool,
    #[serde(default)]
    pub resync: bool,
}

/// A field that tolerates a wrong-typed value by reading as absent/default,
/// mirroring how the previous raw-JSON projection ignored unusable fields.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Lenient<T>(T);

impl<'de, T: Default + serde::de::DeserializeOwned> Deserialize<'de> for Lenient<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        Ok(Self(serde_json::from_value(value).unwrap_or_default()))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LedgerMessage {
    id: Lenient<Option<String>>,
    role: Lenient<Option<String>>,
    content: Lenient<Option<String>>,
    #[serde(alias = "tool_calls")]
    tool_calls: Lenient<Vec<LedgerToolCall>>,
    #[serde(alias = "tool_call_id")]
    tool_call_id: Lenient<Option<String>>,
    #[serde(alias = "tool_name")]
    tool_name: Lenient<Option<String>>,
    #[serde(alias = "is_error")]
    is_error: Lenient<bool>,
}

impl LedgerMessage {
    pub fn id(&self) -> Option<&str> {
        self.id.0.as_deref()
    }

    pub fn role(&self) -> &str {
        self.role.0.as_deref().unwrap_or("")
    }

    pub fn content(&self) -> &str {
        self.content.0.as_deref().unwrap_or("")
    }

    pub fn tool_calls(&self) -> &[LedgerToolCall] {
        &self.tool_calls.0
    }

    pub fn tool_call_id(&self) -> &str {
        self.tool_call_id.0.as_deref().unwrap_or("")
    }

    pub fn tool_name(&self) -> &str {
        self.tool_name.0.as_deref().unwrap_or("tool")
    }

    pub fn is_error(&self) -> bool {
        self.is_error.0
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LedgerToolCall {
    id: Lenient<Option<String>>,
    name: Lenient<Option<String>>,
    /// Raw argument text. An explicitly-null `arguments` is preserved as the
    /// string `"null"` rather than collapsing to the missing-field default
    /// `"{}"`, matching the pre-typed projection.
    #[serde(deserialize_with = "deserialize_json_text")]
    arguments: Option<String>,
    function: Lenient<Option<LedgerFunctionCall>>,
}

impl LedgerToolCall {
    pub fn id(&self) -> &str {
        self.id.0.as_deref().unwrap_or("")
    }

    pub fn name(&self) -> &str {
        self.name
            .0
            .as_deref()
            .or_else(|| self.function.0.as_ref().and_then(|f| f.name.0.as_deref()))
            .unwrap_or("tool")
    }

    pub fn arguments(&self) -> String {
        self.arguments
            .clone()
            .or_else(|| self.function.0.as_ref().and_then(|f| f.arguments.clone()))
            .unwrap_or_else(|| "{}".to_string())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct LedgerFunctionCall {
    name: Lenient<Option<String>>,
    #[serde(deserialize_with = "deserialize_json_text")]
    arguments: Option<String>,
}

/// Read argument text from any JSON value: strings verbatim, everything else as
/// its JSON encoding. Only invoked when the key is present, so an explicit
/// `null` yields `Some("null")` while a missing key stays `None`.
fn deserialize_json_text<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(Some(match value {
        serde_json::Value::String(text) => text,
        other => other.to_string(),
    }))
}

/// Parse recorded tool-call arguments for presentation. Malformed arguments are
/// simply not pre-parsed; the raw string is still rendered by the caller.
pub fn parse_tool_args(args: &str) -> Option<serde_json::Value> {
    serde_json::from_str(args).ok()
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CapabilitySet {
    sync: Option<u64>,
}

impl CapabilitySet {
    fn supports_sync(&self) -> bool {
        self.sync.unwrap_or(0) >= 1
    }
}

/// Whether a child advertises ledger sync, accepting either the top-level
/// `sync` field or a nested `capabilities.sync`.
///
/// Each field is read independently from the already-materialized payload, so a
/// malformed `capabilities` value cannot mask a valid top-level `sync` and no
/// deserializer error is swallowed mid-stream.
pub fn supports_sync(value: &serde_json::Value) -> bool {
    let field = |key: &str| {
        value
            .get(key)
            .cloned()
            .and_then(|field| serde_json::from_value::<CapabilitySet>(field).ok())
    };
    serde_json::from_value::<CapabilitySet>(value.clone())
        .is_ok_and(|capability| capability.supports_sync())
        || field("capabilities").is_some_and(|capability| capability.supports_sync())
}
