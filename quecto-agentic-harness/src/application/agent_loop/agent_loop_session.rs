use super::AgentLoopImpl;
use crate::domain::session_identity::SessionIdentity;

impl AgentLoopImpl {
    /// Adopt the session identity used for provider session IDs, spill
    /// retention and the session-aware tools (D7 #1976: propagated by the
    /// session transactions through the `SessionKeyPropagation` port,
    /// never from a raw string).
    pub fn set_session_key(&mut self, identity: SessionIdentity) {
        self.session_key = identity.runtime_key().to_string();
        self.context_manager.set_session_key(identity.clone());
        self.session_aware_tools()
            .set_session_key(identity.runtime_key());
    }
}
