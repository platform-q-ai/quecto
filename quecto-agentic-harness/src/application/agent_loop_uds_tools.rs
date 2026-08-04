use crate::application::agent_loop::AgentLoopImpl;

impl AgentLoopImpl {
    pub fn can_register_uds_tool_for_owner_with_stable_id(
        &self,
        name: &str,
        owner: &str,
        stable_id: Option<&str>,
    ) -> bool {
        self.extension_tool_registry()
            .can_register_uds_tool_for_owner_with_stable_id(name, owner, stable_id)
    }

    pub fn register_uds_tool_for_owner_with_stable_id(
        &mut self,
        tool: std::sync::Arc<dyn crate::domain::tool::Tool>,
        owner: std::borrow::Cow<'static, str>,
        stable_id: Option<String>,
    ) -> bool {
        self.extension_tool_registry_mut()
            .register_uds_tool_for_owner_with_stable_id(tool, owner, stable_id)
    }
}
