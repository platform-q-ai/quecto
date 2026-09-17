//! Scope admission of a resume target (#2009): the minimum safety rule the
//! resume transaction applies under its claim, for the selection and the
//! exact-key/startup paths alike. Not a second transaction owner: the
//! decision is the domain's, the observations the shared context's.
use crate::application::sessions::dto::ResumeSavedSessionError;
use crate::application::sessions::ports::SessionStore;
use crate::application::sessions::session_home::SessionHomeContext;
use crate::domain::{session_home::SessionHomeScope, session_identity::SessionIdentity};
use std::sync::Arc;

/// The target's claim, held while it is loaded and admitted: released on
/// every refusal or failure after it, kept once the switch commits.
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

/// The caller owns the key while authoritative metadata is re-read; the
/// current facts and the saved home are observed fresh, never cached.
pub(super) async fn admit_home(
    home: &SessionHomeContext,
    identity: &SessionIdentity,
) -> Result<(), ResumeSavedSessionError> {
    let scope = home
        .catalogue
        .read(identity)
        .map_err(ResumeSavedSessionError::Load)?;
    home.admit(&scope)
        .await
        .map_err(ResumeSavedSessionError::Scope)
}

/// A startup identity without a transcript: no home authority means a
/// genuinely new session; any present authority is admitted like a load.
pub(super) async fn admit_new_at_startup(
    home: &SessionHomeContext,
    identity: &SessionIdentity,
) -> Result<(), ResumeSavedSessionError> {
    let scope = home
        .catalogue
        .read(identity)
        .map_err(ResumeSavedSessionError::Load)?;
    match scope {
        SessionHomeScope::LegacyUnscoped => Ok(()),
        SessionHomeScope::Scoped(_) | SessionHomeScope::Unavailable(_) => home
            .admit(&scope)
            .await
            .map_err(ResumeSavedSessionError::Scope),
    }
}
