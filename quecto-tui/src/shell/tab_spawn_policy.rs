//! Parent-policy inheritance for secondary tab agent spawns (#1465 F8).

use std::path::PathBuf;

use super::cli::CliFlags;

/// Parent TUI CLI policy inherited by secondary tab agent spawns.
#[derive(Debug, Clone)]
pub(crate) struct TabSpawnPolicy {
    pub(crate) workflow: bool,
    pub(crate) workflow_guards: bool,
    pub(crate) workflow_disabled: bool,
    pub(crate) config_path: Option<PathBuf>,
    pub(crate) system_prompt: Option<String>,
    pub(crate) disable_tools: Vec<String>,
    /// Secondary tabs persist by default (ADR-0023); parent `--no-persist`
    /// still applies when the operator opted out.
    pub(crate) persist: bool,
}

impl Default for TabSpawnPolicy {
    fn default() -> Self {
        Self {
            workflow: false,
            workflow_guards: false,
            workflow_disabled: false,
            config_path: None,
            system_prompt: None,
            disable_tools: Vec::new(),
            persist: true,
        }
    }
}

impl TabSpawnPolicy {
    pub(crate) fn from_flags(flags: &CliFlags) -> Self {
        Self {
            workflow: flags.workflow,
            workflow_guards: flags.workflow_guards,
            workflow_disabled: flags.workflow_disabled,
            config_path: flags.config_path.clone(),
            system_prompt: flags.system_prompt.clone(),
            disable_tools: flags.disable_tools.clone(),
            persist: flags.persist,
        }
    }
}

/// Build secondary-tab spawn flags from the parent TUI policy (F8).
/// `resume_session` is applied after connect via `resume_session` command —
/// not as a CLI flag — so it is intentionally unused here (F5/F6).
pub(crate) fn tab_spawn_flags_from_policy(
    policy: &TabSpawnPolicy,
    _resume_session: Option<String>,
) -> CliFlags {
    CliFlags {
        socket_path: None,
        workflow: policy.workflow,
        workflow_guards: policy.workflow_guards,
        workflow_disabled: policy.workflow_disabled,
        config_path: policy.config_path.clone(),
        system_prompt: policy.system_prompt.clone(),
        disable_tools: policy.disable_tools.clone(),
        persist: policy.persist,
        kill_on_exit: false,
    }
}

/// Flags for a secondary tab agent spawn with default parent policy.
#[cfg(test)]
pub(crate) fn tab_spawn_flags(_resume_session: Option<String>) -> CliFlags {
    tab_spawn_flags_from_policy(&TabSpawnPolicy::default(), _resume_session)
}

#[cfg(test)]
#[path = "tab_spawn_policy_tests.rs"]
mod tab_spawn_policy_tests;
