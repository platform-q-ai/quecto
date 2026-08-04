use std::borrow::Cow;
use std::sync::Arc;

use crate::domain::tool::Tool;

use super::registration::ToolRegistration;
use super::registry::ToolRegistryImpl;

impl ToolRegistryImpl {
    /// Return whether a UDS-delivered extension tool with `name`, `owner`, and
    /// optional stable ID would be accepted by the registry without mutating it.
    pub fn can_register_uds_tool_for_owner(&self, name: &str, owner: &str) -> bool {
        self.can_register_uds_tool_for_owner_with_stable_id(name, owner, None)
    }

    pub fn can_register_uds_tool_for_owner_with_stable_id(
        &self,
        name: &str,
        owner: &str,
        stable_id: Option<&str>,
    ) -> bool {
        if self.denied_names.contains(name) {
            return false;
        }
        if let Some(existing) = self.metadata.get(name) {
            if !(existing.unloadable && existing.owner.as_ref() == owner) {
                return false;
            }
        }
        let mut metadata = ToolRegistration::uds_owner(owner.to_string());
        if let Some(stable_id) = stable_id {
            metadata = metadata.with_stable_id(stable_id.to_string());
        }
        self.registration_identity_is_available(name, &metadata)
            .is_ok()
    }

    /// Register a UDS-delivered extension tool with per-connection ownership.
    pub fn register_uds_tool_for_owner(
        &mut self,
        tool: Arc<dyn Tool>,
        owner: Cow<'static, str>,
    ) -> bool {
        self.register_uds_tool_for_owner_with_stable_id(tool, owner, None)
    }

    pub fn register_uds_tool_for_owner_with_stable_id(
        &mut self,
        tool: Arc<dyn Tool>,
        owner: Cow<'static, str>,
        stable_id: Option<String>,
    ) -> bool {
        let mut metadata = ToolRegistration::uds_owner(owner);
        if let Some(stable_id) = stable_id {
            metadata = metadata.with_stable_id(stable_id);
        }
        self.register_with_metadata(tool, metadata)
    }
}
