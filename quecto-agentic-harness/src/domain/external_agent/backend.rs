//! Which "brain" a member harness runs (#2287, owner decisions O1 and O2).
//!
//! A `claude_code` member is still a quecto harness process (`quecto agent
//! --mode uds --backend claude-code`), so identity, admission, its endpoint
//! and teardown are unchanged; only its brain is the Claude Code CLI. O1
//! keeps that brain to swarm **workers**: the coordinator launches them into
//! its own container, and they run no workflow and take no effort.

use crate::domain::error::DomainError;
use crate::domain::subagent::{ContainerSelection, SubagentConfig};

/// The brain a member harness runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MemberBackend {
    /// quecto's own agent loop: every launch before #2287.
    #[default]
    Quecto,
    /// The Claude Code CLI, driven over stream-json.
    ClaudeCode,
}

impl MemberBackend {
    /// The spawn tool's `backend` values.
    pub const SPAWN_VALUES: &'static str = "quecto, claude_code";
    /// The agent CLI's `--backend` values.
    pub const FLAG_VALUES: &'static str = "quecto, claude-code";

    /// The backend a spawn tool `backend` value names.
    pub fn from_spawn_value(value: &str) -> Option<Self> {
        match value {
            "quecto" => Some(Self::Quecto),
            "claude_code" => Some(Self::ClaudeCode),
            _ => None,
        }
    }

    /// The backend a `--backend` flag value names.
    pub fn from_flag_value(value: &str) -> Option<Self> {
        match value {
            "quecto" => Some(Self::Quecto),
            "claude-code" => Some(Self::ClaudeCode),
            _ => None,
        }
    }

    /// This backend's `--backend` flag value.
    pub fn flag_value(self) -> &'static str {
        match self {
            Self::Quecto => "quecto",
            Self::ClaudeCode => "claude-code",
        }
    }
}

/// Who is launching, as far as the backend rule asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackendLaunchContext {
    /// The launcher takes part in a swarm: it runs in the swarm's
    /// container, so a launch with `container` omitted lands there.
    pub launcher_is_swarm_participant: bool,
}

/// The refusal for a launcher that is not a swarm participant.
pub const CLAUDE_CODE_WORKERS_ONLY: &str =
    "backend claude_code is only for swarm workers launched by the coordinator into its container";
/// The refusal for a launch into another container.
pub const CLAUDE_CODE_OWN_CONTAINER_ONLY: &str =
    "backend claude_code launches only into the coordinator's own container; omit container";
/// The refusal for a launch that sets `effort`.
pub const CLAUDE_CODE_TAKES_NO_EFFORT: &str = "backend claude_code takes no effort; omit effort";
/// The refusal for a launch that sets `read_only`: the member's tools are
/// its own (#2291), not quecto's.
pub const CLAUDE_CODE_TAKES_NO_READ_ONLY: &str =
    "backend claude_code takes no read_only; omit read_only";
/// The refusal for a launch that sets `disable_tools`.
pub const CLAUDE_CODE_TAKES_NO_DISABLE_TOOLS: &str =
    "backend claude_code takes no disable_tools; omit disable_tools";
/// The refusal for a launch that sets `system`.
pub const CLAUDE_CODE_TAKES_NO_SYSTEM: &str =
    "backend claude_code takes no system prompt; omit system";
/// The refusal for a launch that sets `config`.
pub const CLAUDE_CODE_TAKES_NO_CONFIG: &str = "backend claude_code takes no config; omit config";
/// The refusal for a model of another provider.
pub const CLAUDE_CODE_ANTHROPIC_MODELS_ONLY: &str =
    "backend claude_code runs anthropic models only; omit model or name an anthropic/ model";

/// Whether `config`'s backend may be launched in `context`. `Quecto` always
/// may. `ClaudeCode` may only when every one of these holds (O1): the
/// launcher is a swarm participant; the launch goes into its own container
/// (`container` omitted); no workflow is requested
/// ([`crate::domain::swarm::validate_workflow`]); no `effort` is set; and
/// it sets no field the member would ignore (#2287 review): `read_only`,
/// `disable_tools`, `system`, `config`, or a model of another provider.
/// Each unmet condition has its own refusal, checked in that order.
pub fn validate_backend(
    config: &SubagentConfig,
    context: BackendLaunchContext,
) -> Result<(), DomainError> {
    match config.backend {
        MemberBackend::Quecto => Ok(()),
        MemberBackend::ClaudeCode => claude_code_launch_allowed(config, context),
    }
}

/// `ClaudeCode` is allowed iff every O1 condition holds and every field is
/// one it honours. The destructuring is the allowlist: a field added to
/// [`SubagentConfig`] does not compile here until it is decided.
fn claude_code_launch_allowed(
    config: &SubagentConfig,
    context: BackendLaunchContext,
) -> Result<(), DomainError> {
    let SubagentConfig {
        // Honoured: the first turn, the label and the brain itself.
        task: _,
        agent_id: _,
        backend: _,
        container,
        workflow,
        workflow_guards,
        workflow_spec,
        effort,
        read_only,
        disable_tools,
        system,
        config_path,
        model,
    } = config;
    let refuse = |reason: &str| Err(DomainError::Tool(reason.to_string()));
    match (context.launcher_is_swarm_participant, container) {
        (true, ContainerSelection::Local) => {}
        (true, _) => return refuse(CLAUDE_CODE_OWN_CONTAINER_ONLY),
        (false, _) => return refuse(CLAUDE_CODE_WORKERS_ONLY),
    }
    crate::domain::swarm::validate_workflow(
        true,
        *workflow || *workflow_guards || workflow_spec.is_some(),
    )?;
    let unhonoured = [
        (effort.is_some(), CLAUDE_CODE_TAKES_NO_EFFORT),
        (*read_only, CLAUDE_CODE_TAKES_NO_READ_ONLY),
        (
            !disable_tools.is_empty(),
            CLAUDE_CODE_TAKES_NO_DISABLE_TOOLS,
        ),
        (system.is_some(), CLAUDE_CODE_TAKES_NO_SYSTEM),
        (config_path.is_some(), CLAUDE_CODE_TAKES_NO_CONFIG),
        (
            !anthropic_or_default(model.as_deref()),
            CLAUDE_CODE_ANTHROPIC_MODELS_ONLY,
        ),
    ];
    match unhonoured.iter().find(|(set, _)| *set) {
        Some((_, reason)) => refuse(reason),
        None => Ok(()),
    }
}

/// A model the Claude Code CLI runs: none (its default), or one named
/// `anthropic/<model>`.
fn anthropic_or_default(model: Option<&str>) -> bool {
    match model.map(|model| model.split_once('/')) {
        None => true,
        Some(Some(("anthropic", name))) => !name.is_empty(),
        Some(_) => false,
    }
}

#[cfg(test)]
#[path = "backend_tests.rs"]
mod tests;
