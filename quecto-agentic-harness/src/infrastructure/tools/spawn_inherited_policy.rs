use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::domain::tool_descriptor::ProfileAvailabilityScope;

use super::inherited_tool_policy::{
    InheritedToolPolicySnapshot, WORKFLOW_TOOL_NAME, recorded_scope, workflow_tool_identity,
};

pub(super) type InheritedToolPolicyState = Arc<RwLock<Option<InheritedToolPolicySnapshot>>>;

pub(super) fn new_state() -> InheritedToolPolicyState {
    Arc::new(RwLock::new(None))
}

pub(super) fn replace_state(
    state: &InheritedToolPolicyState,
    snapshot: InheritedToolPolicySnapshot,
) {
    *state
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(snapshot);
}

pub(super) fn set_from_tools(
    state: &InheritedToolPolicyState,
    tools: BTreeMap<String, ProfileAvailabilityScope>,
) {
    replace_state(state, InheritedToolPolicySnapshot::new(tools));
}

pub(super) fn snapshot(state: &InheritedToolPolicyState) -> Option<InheritedToolPolicySnapshot> {
    state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

pub(super) fn tools(
    state: &InheritedToolPolicyState,
) -> Option<BTreeMap<String, ProfileAvailabilityScope>> {
    snapshot(state).map(|snapshot| snapshot.tools)
}

/// #2216: admits a launch's workflow request only when the child could use
/// the workflow tool. A spawn whose `disable_tools` names it, or whose
/// inherited policy records it without child scope, is refused before
/// launch; otherwise the child would start in workflow mode it cannot drive.
pub(super) fn admit_workflow_request(
    state: &InheritedToolPolicyState,
    workflow_requested: bool,
    disable_tools: &[String],
) -> Result<(), String> {
    if workflow_requested {
        let identity = workflow_tool_identity();
        let inputs = identity.resolver_inputs();
        if disable_tools
            .iter()
            .any(|tool| inputs.iter().any(|input| input.as_ref() == tool))
        {
            return Err("workflow and workflow_spec cannot be used while disable_tools denies the workflow tool; drop one of them".into());
        }
        let recorded =
            tools(state).and_then(|tools| recorded_scope(&tools, WORKFLOW_TOOL_NAME, &identity));
        return match recorded {
            None => Ok(()),
            Some(scope) if scope.allows_child() => Ok(()),
            Some(_) => Err("workflow is not available to children of this agent: its tool policy denies the workflow tool to children; spawn without workflow and workflow_spec".into()),
        };
    }
    Ok(())
}

#[cfg(test)]
#[path = "spawn_inherited_policy_workflow_tests.rs"]
mod workflow_tests;
