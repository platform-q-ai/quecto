//! Wire DTOs for the two teardown operations (#1934). Parsed, never interpreted.
//!
//! Compatibility rule: both commands are `deny_unknown_fields`. A field added
//! later is a new command `type`, never an optional field on these two, so an
//! older harness affirmatively rejects a shape it does not understand instead
//! of silently ignoring part of it.
use serde::{Deserialize, Serialize};

/// Byte cap for a teardown command line. This bounds *parsing* of a claimed
/// teardown line: both commands carry a handful of short fields, so anything
/// larger is not one of them. Memory is bounded earlier, by the connection
/// reader's frame cap (`MAX_FRAME_PAYLOAD_BYTES`), before a line reaches
/// this parser.
pub const TEARDOWN_COMMAND_CAP_BYTES: usize = 1024;

pub const SHUTDOWN_COMMAND: &str = "shutdown";
pub const TERMINATE_DELEGATED_AGENT_COMMAND: &str = "terminate_delegated_agent";

/// Inbound teardown command exactly as it appears on the socket.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SubagentTeardownCommand {
    /// Ask the receiving harness to shut itself and its subtree down.
    Shutdown {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        reason: String,
    },
    /// Ask the receiving harness to resolve the next edge toward a descendant.
    TerminateDelegatedAgent {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        target_uuid: String,
        target_generation: u64,
        remaining_depth: u32,
    },
}

impl SubagentTeardownCommand {
    pub fn id(&self) -> Option<&str> {
        match self {
            Self::Shutdown { id, .. } | Self::TerminateDelegatedAgent { id, .. } => id.as_deref(),
        }
    }

    pub const fn command_name(&self) -> &'static str {
        match self {
            Self::Shutdown { .. } => SHUTDOWN_COMMAND,
            Self::TerminateDelegatedAgent { .. } => TERMINATE_DELEGATED_AGENT_COMMAND,
        }
    }
}

/// Why a line is not a well-formed teardown command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    /// A claimed teardown line longer than the parse cap. `id` is the
    /// correlation id when one could be read, so the rejection is matched.
    Oversized {
        id: Option<String>,
        bytes: usize,
        cap: usize,
    },
    /// Not claimed: not JSON, no object, no `type`, or a `type` that is some
    /// other protocol command. The caller must dispatch it as usual and must
    /// not answer it here.
    NotATeardownCommand,
    /// Claimed `type` but not the declared shape. `id` is the correlation id
    /// when the line carried one (rendered as text if it was not a string),
    /// so the rejection can still be matched.
    Malformed { id: Option<String>, detail: String },
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Oversized { bytes, cap, .. } => {
                write!(f, "teardown command of {bytes} bytes exceeds cap {cap}")
            }
            Self::NotATeardownCommand => f.write_str("not a teardown command"),
            Self::Malformed { detail, .. } => write!(f, "malformed teardown command: {detail}"),
        }
    }
}

/// The only two `type` values this edge claims. Everything else belongs to
/// the ordinary dispatcher and gets no frame from here.
fn claimed(kind: &str) -> bool {
    kind == SHUTDOWN_COMMAND || kind == TERMINATE_DELEGATED_AGENT_COMMAND
}

/// Parse one socket line.
///
/// Claiming comes first: a line is only ours when it is a JSON object whose
/// `type` is one of the two commands; anything else is
/// [`WireError::NotATeardownCommand`] and gets no response from this edge.
/// The id is then read leniently (a string is kept; a non-string is a
/// rejection that still echoes the id as text), the parse cap is applied,
/// and finally the strict shape.
pub fn parse_teardown_command(line: &str) -> Result<SubagentTeardownCommand, WireError> {
    let line = line.trim_end_matches(['\r', '\n']);
    // Lenient claim read: a generic object so a duplicated key keeps its
    // last value instead of failing the claim.
    let Ok(serde_json::Value::Object(object)) = serde_json::from_str::<serde_json::Value>(line)
    else {
        return Err(WireError::NotATeardownCommand);
    };
    if !object
        .get("type")
        .and_then(serde_json::Value::as_str)
        .is_some_and(claimed)
    {
        return Err(WireError::NotATeardownCommand);
    }
    let id = match object.get("id") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(id)) => Some(id.clone()),
        Some(other) => {
            return Err(WireError::Malformed {
                id: Some(other.to_string()),
                detail: "id must be a string".into(),
            });
        }
    };
    if line.len() > TEARDOWN_COMMAND_CAP_BYTES {
        return Err(WireError::Oversized {
            id,
            bytes: line.len(),
            cap: TEARDOWN_COMMAND_CAP_BYTES,
        });
    }
    serde_json::from_str(line).map_err(|error| WireError::Malformed {
        id,
        detail: error.to_string(),
    })
}

/// Outbound response for a teardown command, correlated by `id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeardownResponse {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub command: String,
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<TeardownResponseData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TeardownResponseData {
    /// Shutdown admitted; the harness will now tear down and exit.
    ShuttingDown { reason: String },
    /// The direct child that is the target was asked to shut down.
    ShutdownRequested { child_uuid: String },
    /// The command was forwarded one hop; this harness stays alive.
    Forwarded {
        via_uuid: String,
        remaining_depth: u32,
    },
}

impl TeardownResponse {
    pub fn ok(id: Option<&str>, command: &'static str, data: TeardownResponseData) -> Self {
        Self {
            kind: "response".to_owned(),
            id: id.map(str::to_owned),
            command: command.to_owned(),
            success: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn err(id: Option<&str>, command: &'static str, error: impl Into<String>) -> Self {
        Self {
            kind: "response".to_owned(),
            id: id.map(str::to_owned),
            command: command.to_owned(),
            success: false,
            data: None,
            error: Some(error.into()),
        }
    }

    /// The response as one newline-delimited JSON line. Adapters that frame
    /// per the connection's wire mode serialize the value themselves; this
    /// is the legacy line shape.
    pub fn to_line(&self) -> String {
        let mut line = serde_json::to_string(self).expect("teardown response is serializable");
        line.push('\n');
        line
    }
}

#[cfg(test)]
#[path = "wire_tests.rs"]
mod tests;
