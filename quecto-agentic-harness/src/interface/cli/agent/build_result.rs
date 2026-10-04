//! What building an agent from config yields (split from `agent.rs`, which
//! is at its size limit).
use std::sync::Arc;

use super::ExtensionRegistry;
use crate::application::agent_loop::AgentLoopImpl;
use crate::domain::agents::configured_extensions::{AgentExtensions, AgentRole};
use crate::infrastructure::config::Config;
use crate::infrastructure::config::extensions::ExtensionConfig;

use super::{NotificationRx, SharedHarnessLifecycle, SubagentRegistry};

pub(crate) struct AgentBuildResult {
    pub agent: AgentLoopImpl,
    /// The run's catalogue handles (#1845, #1848), the same instances the
    /// spawn tool and the startup effort admission used.
    pub catalogue: crate::interface::cli::catalogue_handles::CatalogueHandles,
    /// The run's retained-context handles (D9 #1978): the store the loop's
    /// session recovers through and the scrub the ephemeral exit reaches.
    pub retention: crate::interface::cli::retention_handles::RetentionHandles,
    pub workflow_config: Option<crate::domain::workflow::WorkflowConfig>,
    pub extension_prompt_snippets: String,
    pub model: String,
    pub ext_registry: Arc<std::sync::Mutex<ExtensionRegistry>>,
    pub notification_rx: Option<NotificationRx>,
    pub subagent_registry: Option<SubagentRegistry>,
    pub harness_lifecycle: Option<SharedHarnessLifecycle>,
    /// The environment control slot (#2070) the loop hands its teardown.
    pub environment_control:
        Option<crate::infrastructure::tools::agent_cmd_containers::EnvironmentControlSlot>,
    pub workflow_state: Option<crate::interface::shared::WorkflowStateHandle>, // #562
    pub workspace: std::path::PathBuf,
    /// `telemetry.event_log.enabled` (#2150).
    pub event_log: bool,
    /// The configured extensions this agent launches (#2446).
    pub extensions: AgentExtensions,
}

/// The configured extensions an agent may launch (#2446): a top-level
/// agent's own global config's; a spawned child's, only the validated list
/// its parent handed down with its policy. A child's own `--config` (which
/// a spawn call may name) never names commands to run.
fn configured(config: &Config, flags: &super::AgentFlags) -> Vec<ExtensionConfig> {
    match flags.spawned {
        true => flags
            .inherited_tool_policy
            .as_ref()
            .map(|policy| policy.extensions.clone())
            .unwrap_or_default(),
        false => config.extensions.clone(),
    }
}

/// The configured extensions an agent started with `flags` launches: a
/// spawned child's under its own id; none under `--no-extensions`.
pub(super) fn agent_extensions(config: &Config, flags: &super::AgentFlags) -> AgentExtensions {
    AgentExtensions::select(
        configured(config, flags)
            .iter()
            .map(ExtensionConfig::spec)
            .collect(),
        AgentRole::of(flags.spawned),
        flags.session_name.as_deref(),
        flags.launch_extensions,
    )
}

/// The configured extensions each local child of this agent launches:
/// those of its own marked `children`; none under `--no-extensions`.
pub(super) fn child_extensions(config: &Config, flags: &super::AgentFlags) -> Vec<ExtensionConfig> {
    configured(config, flags)
        .into_iter()
        .filter(|extension| flags.launch_extensions && extension.children)
        .collect()
}
