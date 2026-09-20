//! Startup admission of the loop's own composed session (#2009): the explicit
//! resume's scope rule worded for the command line (only a session that lives
//! in ANOTHER folder is told to go there), and the orphan rule for a lone home.
use crate::application::sessions::dto::resume_saved_session::ResumeDisposition;
use crate::application::sessions::dto::{ResumeSavedSessionError, StartupRefusal};
use crate::application::sessions::session_home::SessionHomeContext;
use crate::domain::{session_home::SessionHomeScope, session_identity::SessionIdentity};
pub(super) async fn admit_at_startup(
    home: &SessionHomeContext,
    identity: &SessionIdentity,
) -> Result<(), ResumeSavedSessionError> {
    let scope = home
        .catalogue
        .read(identity)
        .map_err(ResumeSavedSessionError::Load)?;
    home.admit(&scope).await.map_err(|disposition| {
        let elsewhere = matches!(disposition, ResumeDisposition::DifferentExecutionDirectory);
        let execution_dir = match &scope {
            SessionHomeScope::Scoped(saved) if elsewhere => Some(saved.execution_dir.clone()),
            _ => None,
        };
        ResumeSavedSessionError::StartupScope(StartupRefusal {
            key: identity.runtime_key().to_string(),
            disposition,
            execution_dir,
        })
    })
}
/// A startup identity without a transcript is a new session whatever sidecar
/// is beside it: a home there is an orphan (a save that never committed, a
/// transcript removed by hand), discarded under the claim so a key with no
/// history never refuses to start and a stale home is never inherited.
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
        SessionHomeScope::Scoped(_) | SessionHomeScope::Unavailable(_) => {
            tracing::info!(
                target: "session_home",
                session = %identity.runtime_key(),
                "home without a transcript at startup; discarded as an orphan"
            );
            home.catalogue
                .discard_orphan(identity)
                .map_err(ResumeSavedSessionError::Load)
        }
    }
}
