use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use crate::domain::tool_descriptor::ProfileAvailabilityScope;

use super::inherited_tool_policy::InheritedToolPolicySnapshot;

pub(super) type InheritedToolPolicyState = Arc<RwLock<Option<InheritedToolPolicySnapshot>>>;

pub(super) fn new_state() -> InheritedToolPolicyState {
    Arc::new(RwLock::new(None))
}

pub(super) fn replace_state(
    state: &InheritedToolPolicyState,
    snapshot: InheritedToolPolicySnapshot,
) {
    *state.write().expect("inherited policy lock") = Some(snapshot);
}

pub(super) fn set_from_tools(
    state: &InheritedToolPolicyState,
    tools: BTreeMap<String, ProfileAvailabilityScope>,
) {
    replace_state(state, InheritedToolPolicySnapshot::new(tools));
}

pub(super) fn snapshot(state: &InheritedToolPolicyState) -> Option<InheritedToolPolicySnapshot> {
    state.read().expect("inherited policy lock").clone()
}

pub(super) fn tools(
    state: &InheritedToolPolicyState,
) -> Option<BTreeMap<String, ProfileAvailabilityScope>> {
    snapshot(state).map(|snapshot| snapshot.tools)
}
