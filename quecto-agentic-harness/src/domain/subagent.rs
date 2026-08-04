// Subagent domain types: configuration and validation.

use std::path::PathBuf;

use super::error::DomainError;
use super::ids::AgentUuid;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayNameResolutionEntry {
    pub agent_uuid: AgentUuid,
    pub display_name: String,
    pub live: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplayNameResolveError {
    NoLiveMatch { display_name: String },
    AmbiguousLiveMatch { display_name: String },
}

pub fn resolve_live_display_name(
    entries: &[DisplayNameResolutionEntry],
    display_name: &str,
) -> Result<AgentUuid, DisplayNameResolveError> {
    let mut matches = entries
        .iter()
        .filter(|entry| entry.live && entry.display_name == display_name)
        .map(|entry| entry.agent_uuid.clone());

    let Some(first) = matches.next() else {
        return Err(DisplayNameResolveError::NoLiveMatch {
            display_name: display_name.to_string(),
        });
    };

    if matches.next().is_some() {
        return Err(DisplayNameResolveError::AmbiguousLiveMatch {
            display_name: display_name.to_string(),
        });
    }

    Ok(first)
}

pub fn assert_display_name_available_for_spawn(
    entries: &[DisplayNameResolutionEntry],
    display_name: &str,
) -> Result<(), DisplayNameResolveError> {
    match resolve_live_display_name(entries, display_name) {
        Ok(_) | Err(DisplayNameResolveError::AmbiguousLiveMatch { .. }) => {
            Err(DisplayNameResolveError::AmbiguousLiveMatch {
                display_name: display_name.to_string(),
            })
        }
        Err(DisplayNameResolveError::NoLiveMatch { .. }) => Ok(()),
    }
}

/// Configuration for spawning a subagent.
#[derive(Debug, Clone)]
pub struct SubagentConfig {
    /// The task to execute (optional — agent starts idle if omitted).
    pub task: Option<String>,
    /// Optional user-facing display label (`agent_id` on the compatibility wire).
    /// This is not durable identity and must not key persistence or sockets.
    pub agent_id: Option<String>,
    /// Whether the subagent should restrict to workspace.
    pub restrict_to_workspace: bool,
    /// Optional system prompt for the subagent.
    pub system: Option<String>,
    /// Optional config file path to forward as `--config <path>`.
    pub config_path: Option<PathBuf>,
    /// Whether to start the child with `--workflow`.
    pub workflow: bool,
    /// Whether to start the child with `--workflow-guards`.
    pub workflow_guards: bool,
    /// Optional by-value workflow assignment. When set, the child is launched
    /// with `--workflow-spec <path>` and runs exactly that template in Active
    /// mode (binding).
    pub workflow_spec: Option<crate::domain::workflow::WorkflowSpec>,
    /// Optional model override, forwarded to the child as `--model <value>`.
    /// Resolved to the canonical `provider/model` form. When `None`, the child
    /// resolves its model from the inherited `--config` or the built-in default.
    pub model: Option<String>,
    /// Optional reasoning effort override, forwarded to the child as `--effort <value>`.
    /// When `None`, the child resolves effort from config, environment, or provider default.
    pub effort: Option<String>,
    /// Tool names to disable and hide from the child's model-visible definitions
    /// before its session starts (forwarded as `--disable-tool <name>` per entry).
    /// Empty means no tools are disabled. Used to launch read-only children
    /// (e.g. reviewers) with `write` and `edit` disabled so the model never sees
    /// them (#957/#1276).
    pub disable_tools: Vec<String>,
    /// Whether this sub-agent was spawned read-only — i.e. both `write` and
    /// `edit` are disabled (via `read_only: true` or an equivalent
    /// `disable_tools` set). Surfaced to the TUI so the left panel can mark the
    /// agent as an observer (#966). Purely a display flag; enforcement is #957.
    pub read_only: bool,
}

/// A validated model argument, in either of the two forms accepted by
/// `set_model` and `spawn`: a single `provider/model` string, or a separate
/// `provider` + `model_id` pair.
///
/// This is the single source of truth for model-argument validation so the
/// `spawn` tool and `agent_cmd set_model` cannot diverge (#881).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelArg {
    /// A full `provider/model` string (e.g. `openai/gpt-5.5`).
    Full(String),
    /// A separate provider + model id pair (e.g. `openai` + `gpt-5.5`).
    Pair { provider: String, model_id: String },
}

impl ModelArg {
    /// Collapse to the canonical single-string `provider/model` form used by
    /// the `--model` CLI flag and the config default.
    pub fn to_model_string(&self) -> String {
        match self {
            ModelArg::Full(m) => m.clone(),
            ModelArg::Pair { provider, model_id } => format!("{provider}/{model_id}"),
        }
    }
}

/// Validate the `model` / `provider` / `model_id` argument trio shared by
/// `set_model` and `spawn`. Empty strings are treated as absent. Exactly one of
/// (`model`) or (`provider` + `model_id`) must be supplied.
///
/// Returns `Ok(None)` when none are supplied (caller falls back to its default
/// behaviour) and `Err` with a clear message when the combination is invalid.
pub fn parse_model_arg(
    model: Option<&str>,
    provider: Option<&str>,
    model_id: Option<&str>,
) -> Result<Option<ModelArg>, String> {
    let model = model.map(str::trim).filter(|s| !s.is_empty());
    let provider = provider.map(str::trim).filter(|s| !s.is_empty());
    let model_id = model_id.map(str::trim).filter(|s| !s.is_empty());

    match (model, provider, model_id) {
        (Some(m), _, _) => Ok(Some(ModelArg::Full(m.to_string()))),
        (None, Some(p), Some(mid)) => Ok(Some(ModelArg::Pair {
            provider: p.to_string(),
            model_id: mid.to_string(),
        })),
        (None, Some(_), None) => Err("provider requires model_id".to_string()),
        (None, None, Some(_)) => Err("model_id requires provider".to_string()),
        (None, None, None) => Ok(None),
    }
}

/// Validate an agent_id against an allowlist.
/// Returns Ok if the agent_id is in the allowlist, or Err if not.
pub fn validate_agent_id(agent_id: &str, allowlist: &[String]) -> Result<(), DomainError> {
    if allowlist.iter().any(|id| id == agent_id) {
        Ok(())
    } else {
        Err(DomainError::Security(format!(
            "agent_id '{}' is not allowed",
            agent_id
        )))
    }
}

#[cfg(test)]
#[path = "subagent_tests.rs"]
mod tests;
