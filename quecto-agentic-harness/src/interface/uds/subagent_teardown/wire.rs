//! Wire DTOs for the two teardown operations (#1934). Parsed, never interpreted.
use serde::{Deserialize, Serialize};

/// Byte cap for a teardown command line. Both commands carry a handful of
/// short fields; anything larger is not one of them.
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
    Oversized {
        bytes: usize,
        cap: usize,
    },
    /// Well-formed JSON whose `type` is some other protocol command.
    NotATeardownCommand,
    /// Recognised `type` but not the declared shape. `id` is the correlation
    /// id when the line carried one, so the rejection can still be matched.
    Malformed {
        id: Option<String>,
        detail: String,
    },
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Oversized { bytes, cap } => {
                write!(f, "teardown command of {bytes} bytes exceeds cap {cap}")
            }
            Self::NotATeardownCommand => f.write_str("not a teardown command"),
            Self::Malformed { detail, .. } => write!(f, "malformed teardown command: {detail}"),
        }
    }
}

/// Parse one socket line. The cap is checked before any allocation; the
/// `type` allowlist is checked before the full shape so an unrelated command
/// is reported as such rather than as malformed.
pub fn parse_teardown_command(line: &str) -> Result<SubagentTeardownCommand, WireError> {
    let line = line.trim_end_matches(['\r', '\n']);
    if line.len() > TEARDOWN_COMMAND_CAP_BYTES {
        return Err(WireError::Oversized {
            bytes: line.len(),
            cap: TEARDOWN_COMMAND_CAP_BYTES,
        });
    }
    #[derive(Deserialize)]
    struct Tag<'a> {
        #[serde(borrow, rename = "type")]
        kind: &'a str,
        #[serde(default)]
        id: Option<String>,
    }
    let tag: Tag<'_> = serde_json::from_str(line).map_err(|error| WireError::Malformed {
        id: None,
        detail: error.to_string(),
    })?;
    let recognised = tag.kind == SHUTDOWN_COMMAND || tag.kind == TERMINATE_DELEGATED_AGENT_COMMAND;
    if !recognised {
        return Err(WireError::NotATeardownCommand);
    }
    serde_json::from_str(line).map_err(|error| WireError::Malformed {
        id: tag.id,
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

    /// One newline-terminated frame, ready to write and flush.
    pub fn to_line(&self) -> String {
        let mut line = serde_json::to_string(self).expect("teardown response is serializable");
        line.push('\n');
        line
    }
}

#[cfg(test)]
#[path = "wire_tests.rs"]
mod tests;
