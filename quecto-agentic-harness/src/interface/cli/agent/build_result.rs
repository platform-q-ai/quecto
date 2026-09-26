//! What building an agent from config yields (split from `agent.rs`, which
//! is at its size limit).
use std::sync::Arc;

use super::ExtensionRegistry;
use crate::application::agent_loop::AgentLoopImpl;

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
}
