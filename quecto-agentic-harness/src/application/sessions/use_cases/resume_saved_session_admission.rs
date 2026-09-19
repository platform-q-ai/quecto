//! Scope admission of a resume target (#2009): the minimum safety rule the
//! resume transaction applies under its claim, for the selection and the
//! exact-key/startup paths alike. Not a second transaction owner: the
//! decision is the domain's, the observations the shared context's.
use crate::application::sessions::ports::SessionStore;
use crate::domain::session_identity::SessionIdentity;
use std::sync::Arc;

/// The target's claim, held while it is loaded and admitted: released on every
/// failure after it unless the caller clears `release` (own key, #1995); kept on commit.
pub(super) struct PendingClaim {
    store: Arc<dyn SessionStore>,
    identity: SessionIdentity,
    pub(super) release: bool,
}
impl PendingClaim {
    pub(super) fn new(store: Arc<dyn SessionStore>, identity: SessionIdentity) -> Self {
        Self {
            store,
            identity,
            release: true,
        }
    }
}
impl Drop for PendingClaim {
    fn drop(&mut self) {
        if self.release {
            self.store.release(&self.identity);
        }
    }
}
