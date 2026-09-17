//! Home acquisition of a new persistent identity (#2009), on the save
//! transaction's existing path: no competing save owner, and no home
//! failure ever costs a transcript.
use crate::application::sessions::dto::SaveSessionError;
use crate::application::sessions::ports::SessionStore;
use crate::application::sessions::session_home::SessionHomeContext;
use crate::domain::session_identity::SessionIdentity;

/// A new persistent identity acquires its home before its first transcript
/// write. Discovery that fails (no Git, a mount boundary, an unreadable
/// cwd) never loses the transcript: the record is written without a home
/// — legacy-unscoped, observable in every listing — and the failure is
/// surfaced as a diagnostic.
pub(super) async fn prepare_home(
    home: Option<&SessionHomeContext>,
    store: &dyn SessionStore,
    identity: &SessionIdentity,
) -> Result<(), SaveSessionError> {
    let Some(home) = home else {
        return Ok(());
    };
    store.claim(identity).map_err(SaveSessionError::Store)?;
    if SessionStore::exists(store, identity)
        .await
        .map_err(SaveSessionError::Store)?
    {
        return Ok(());
    }
    if let Err(error) = home.record_new(identity).await {
        tracing::warn!(
            target: "session_home",
            session = %identity.runtime_key(),
            error = %error,
            "workspace home unavailable; transcript saved without a home (legacy-unscoped)"
        );
    }
    Ok(())
}
