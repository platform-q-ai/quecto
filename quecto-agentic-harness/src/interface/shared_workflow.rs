//! Workflow tool registration for the shared runtime (split from `shared.rs`
//! for the 750-line cap).

use super::WorkflowStateHandle;

/// Register the workflow tool and optional guard in a tool registry.
///
/// Tool construction goes through the bundled native provider seam (#1276 Phase 3)
/// while engine-handle sharing and optional guard registration stay identical.
#[cfg(any(test, feature = "test-support"))]
pub fn register_workflow_tool(
    registry: &mut crate::infrastructure::tools::registry::ToolRegistryImpl,
    wf_config: crate::domain::workflow::WorkflowConfig,
    guards_enabled: bool,
    event_emitter: Option<crate::infrastructure::tools::workflow_tool::WorkflowEventEmitter>,
) -> Result<WorkflowStateHandle, crate::domain::workflow::WorkflowError> {
    register_workflow_tool_with_participation(
        registry,
        wf_config,
        guards_enabled,
        event_emitter,
        crate::infrastructure::tools::swarm_bridge::Participation::none(),
    )
}

/// `participation` is the composition's shared swarm handle (#1715): the
/// workflow tool refuses to act once this container hosts a swarm run.
pub fn register_workflow_tool_with_participation(
    registry: &mut crate::infrastructure::tools::registry::ToolRegistryImpl,
    wf_config: crate::domain::workflow::WorkflowConfig,
    guards_enabled: bool,
    event_emitter: Option<crate::infrastructure::tools::workflow_tool::WorkflowEventEmitter>,
    participation: crate::infrastructure::tools::swarm_bridge::Participation,
) -> Result<WorkflowStateHandle, crate::domain::workflow::WorkflowError> {
    use crate::infrastructure::extensions::native::{
        WorkflowToolDeps, build_workflow_tool_extension, register_bundled_native_tools,
    };

    let engine: WorkflowStateHandle = std::sync::Arc::new(std::sync::Mutex::new(
        crate::domain::workflow::WorkflowEngine::new(wf_config, guards_enabled)?,
    ));

    register_bundled_native_tools(
        registry,
        vec![build_workflow_tool_extension(WorkflowToolDeps {
            engine: engine.clone(),
            event_emitter,
            participation,
        })],
    );

    if guards_enabled {
        let guard = crate::infrastructure::tools::workflow_tool::WorkflowGuard::new(engine.clone());
        registry.register_guard(std::sync::Arc::new(guard));
    }

    Ok(engine)
}
