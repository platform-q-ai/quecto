use super::AgentLoopImpl;

impl AgentLoopImpl {
    /// Switch the session key used for provider session IDs and spill recall.
    pub fn set_session_key(&mut self, session_key: String) {
        self.session_key = session_key.clone();
        self.context_manager.set_session_key(session_key.clone());
        self.tool_registry.set_session_key(&session_key);
    }
}
