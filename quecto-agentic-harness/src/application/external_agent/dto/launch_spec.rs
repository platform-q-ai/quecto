//! What starting an external agent takes (#2286): the flags its process is
//! given, where it runs, the member's own state directory and the one
//! credential that reaches it. Later slices fill the MCP config (S5), the
//! settings and tools (S7) and the budget (S6); the credential mode comes
//! with S9.

use std::path::PathBuf;

use serde_json::Value;

/// One start of an external agent process.
#[derive(Debug, Clone, PartialEq)]
pub struct ExternalAgentLaunchSpec {
    /// The model the agent runs (`--model`).
    pub model: String,
    /// The agent's own tools (`--tools`); empty turns them all off.
    pub tools: Vec<String>,
    /// The MCP servers it may reach (`--mcp-config`): a JSON object.
    pub mcp_config: Value,
    /// Its per-invocation settings (`--settings`): a JSON object.
    pub settings: Value,
    /// The spend cap within a turn (`--max-budget-usd`), in US dollars.
    pub max_budget_usd: f64,
    /// The checkout the agent works in: its working directory.
    pub checkout: PathBuf,
    /// The member's own state directory: its private `HOME` and agent
    /// config directory are made inside it.
    pub member_dir: PathBuf,
    /// The one credential the agent is given.
    pub credential: CredentialEnv,
}

/// The one credential variable an external agent is given. Its value never
/// appears in `Debug` output.
#[derive(Clone, PartialEq, Eq)]
pub struct CredentialEnv {
    pub name: String,
    pub value: String,
}

impl std::fmt::Debug for CredentialEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialEnv")
            .field("name", &self.name)
            .field("value", &"<redacted>")
            .finish()
    }
}

#[cfg(test)]
#[path = "launch_spec_tests.rs"]
mod tests;
