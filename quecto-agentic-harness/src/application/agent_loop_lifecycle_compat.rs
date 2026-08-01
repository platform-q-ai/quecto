use std::{borrow::Cow, sync::Arc};

use crate::{application::agent_loop::AgentLoopImpl, domain::tool::Tool};

impl AgentLoopImpl {
    pub fn tool_registry_extension_names(&self) -> Vec<String> {
        self.runtime_tool_names()
    }

    pub fn register_extension_tool(&mut self, tool: Arc<dyn Tool>) -> bool {
        self.register_runtime_tool(tool)
    }

    pub fn register_uds_extension_tool(&mut self, tool: Arc<dyn Tool>) -> bool {
        self.register_uds_tool(tool)
    }

    pub fn can_register_uds_extension_tool_for_owner(&self, name: &str, owner: &str) -> bool {
        self.can_register_uds_tool_for_owner(name, owner)
    }

    pub fn register_uds_extension_tool_for_owner(
        &mut self,
        tool: Arc<dyn Tool>,
        owner: Cow<'static, str>,
    ) -> bool {
        self.register_uds_tool_for_owner(tool, owner)
    }

    pub fn unregister_extension_tool(&mut self, name: &str) {
        self.unregister_runtime_tool(name)
    }

    pub fn unregister_uds_extension_tools_for_client(&mut self, client_id: u64) -> Vec<String> {
        self.unregister_uds_tools_for_client(client_id)
    }
}
