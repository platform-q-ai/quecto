//! Capabilities schema for WASM tool sidecar files.
//!
//! Each WASM tool ships a `<name>.capabilities.json` file declaring what
//! host resources it needs. The runtime enforces these declarations.

use serde::{Deserialize, Serialize};

/// Capabilities declared by a WASM tool.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolCapabilities {
    /// HTTP capabilities (allowlist, rate limits).
    #[serde(default)]
    pub http: HttpCapabilities,
    /// Workspace filesystem access.
    #[serde(default)]
    pub workspace: WorkspaceCapabilities,
    /// Channel / messaging access.
    #[serde(default)]
    pub channel: ChannelCapabilities,
    /// Cron store access.
    #[serde(default)]
    pub cron: bool,
    /// Spill store access.
    #[serde(default)]
    pub spill: bool,
}

/// HTTP capabilities for a WASM tool.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HttpCapabilities {
    /// Allowed hostnames for outbound HTTP requests.
    #[serde(default)]
    pub allowlist: Vec<HttpAllowlistEntry>,
    /// Per-execution rate limits.
    #[serde(default)]
    pub rate_limit: Option<HttpRateLimit>,
}

/// A single HTTP allowlist entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpAllowlistEntry {
    /// The hostname to allow (e.g., "api.brave.com").
    pub host: String,
    /// Optional path prefix restriction.
    #[serde(default)]
    pub path_prefix: Option<String>,
    /// Allowed HTTP methods (empty = all methods allowed).
    #[serde(default)]
    pub methods: Vec<String>,
}

/// HTTP rate limit configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpRateLimit {
    /// Maximum requests per execution.
    #[serde(default = "default_requests_per_execution")]
    pub requests_per_execution: u32,
}

fn default_requests_per_execution() -> u32 {
    50
}

/// Workspace filesystem capabilities.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkspaceCapabilities {
    /// Whether the tool can read workspace files.
    #[serde(default)]
    pub read: bool,
    /// Whether the tool can write workspace files.
    #[serde(default)]
    pub write: bool,
    /// Allowed path prefixes (empty = all paths within workspace).
    #[serde(default)]
    pub allowed_prefixes: Vec<String>,
}

/// Channel / messaging capabilities.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChannelCapabilities {
    /// Whether the tool can send messages.
    #[serde(default)]
    pub send: bool,
}

impl ToolCapabilities {
    /// Parse capabilities from a JSON string.
    pub fn from_json(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|e| format!("invalid capabilities JSON: {e}"))
    }

    /// Serialize capabilities to a JSON string.
    pub fn to_json(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self).map_err(|e| format!("failed to serialize: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_capabilities() {
        let caps = ToolCapabilities::default();
        assert!(!caps.cron);
        assert!(!caps.spill);
        assert!(caps.http.allowlist.is_empty());
        assert!(!caps.workspace.read);
        assert!(!caps.workspace.write);
        assert!(!caps.channel.send);
    }

    #[test]
    fn test_parse_minimal_json() {
        let json = r#"{}"#;
        let caps = ToolCapabilities::from_json(json).unwrap();
        assert!(!caps.cron);
    }

    #[test]
    fn test_parse_full_json() {
        let json = r#"{
            "http": {
                "allowlist": [
                    {"host": "api.brave.com", "path_prefix": "/search", "methods": ["GET"]}
                ],
                "rate_limit": {"requests_per_execution": 10}
            },
            "workspace": {"read": true, "write": false, "allowed_prefixes": ["data/"]},
            "channel": {"send": true},
            "cron": true,
            "spill": true
        }"#;
        let caps = ToolCapabilities::from_json(json).unwrap();
        assert!(caps.cron);
        assert!(caps.spill);
        assert!(caps.channel.send);
        assert!(caps.workspace.read);
        assert!(!caps.workspace.write);
        assert_eq!(caps.workspace.allowed_prefixes, vec!["data/"]);
        assert_eq!(caps.http.allowlist.len(), 1);
        assert_eq!(caps.http.allowlist[0].host, "api.brave.com");
        assert_eq!(
            caps.http
                .rate_limit
                .as_ref()
                .unwrap()
                .requests_per_execution,
            10
        );
    }

    #[test]
    fn test_parse_invalid_json() {
        let result = ToolCapabilities::from_json("not json");
        assert!(result.is_err());
    }

    #[test]
    fn test_roundtrip() {
        let caps = ToolCapabilities {
            cron: true,
            spill: false,
            ..Default::default()
        };
        let json = caps.to_json().unwrap();
        let parsed = ToolCapabilities::from_json(&json).unwrap();
        assert!(parsed.cron);
        assert!(!parsed.spill);
    }
}
