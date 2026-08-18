use std::borrow::Cow;
use std::sync::Arc;

use super::registry::ToolRegistryImpl;
use crate::domain::tool::Tool;

impl ToolRegistryImpl {
    /// Compatibility name for the legacy extension lifecycle API.
    pub fn extension_names(&self) -> Vec<String> {
        self.runtime_tool_names()
    }

    /// Compatibility name for the legacy extension lifecycle API.
    pub fn register_extension(&mut self, tool: Arc<dyn Tool>) -> bool {
        self.register_runtime_tool(tool)
    }

    /// Compatibility name for the legacy UDS lifecycle API.
    pub fn register_uds_extension(&mut self, tool: Arc<dyn Tool>) -> bool {
        self.register_uds_tool(tool)
    }

    /// Compatibility name for the legacy UDS lifecycle API.
    pub fn can_register_uds_extension_for_owner(&self, name: &str, owner: &str) -> bool {
        self.can_register_uds_tool_for_owner(name, owner)
    }

    /// Compatibility name for the legacy UDS lifecycle API.
    pub fn register_uds_extension_for_owner(
        &mut self,
        tool: Arc<dyn Tool>,
        owner: Cow<'static, str>,
    ) -> bool {
        self.register_uds_tool_for_owner(tool, owner)
    }

    /// Compatibility name for the legacy extension lifecycle API.
    pub fn unregister_extension(&mut self, name: &str) {
        self.unregister_runtime_tool(name)
    }

    /// Compatibility name for the legacy extension lifecycle API.
    pub fn unregister_extensions_for_owner(&mut self, owner: &str) -> Vec<String> {
        self.unregister_runtime_tools_for_owner(owner)
    }
}
