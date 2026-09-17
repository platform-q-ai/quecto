//! Startup admission of the loop's own composed session (#2009): the same
//! scope rule as an explicit resume, worded for the command line, and the
//! orphan rule for a key that has a home sidecar but no transcript.
use super::resume_saved_session_admission::admit_home;
use crate::application::sessions::dto::{ResumeSavedSessionError, StartupRefusal};
use crate::application::sessions::session_home::SessionHomeContext;
use crate::domain::{session_home::SessionHomeScope, session_identity::SessionIdentity};

/// The loaded transcript's home must admit the current directory. A scope
/// refusal names the key and what the user can do now; every other failure
/// is reported as it is.
pub(super) async fn admit_at_startup(
    home: &SessionHomeContext,
    identity: &SessionIdentity,
) -> Result<(), ResumeSavedSessionError> {
    admit_home(home, identity)
        .await
        .map_err(|error| match error {
            ResumeSavedSessionError::Scope(disposition) => {
                ResumeSavedSessionError::StartupScope(StartupRefusal {
                    key: identity.runtime_key().to_string(),
                    disposition,
                })
            }
            other => other,
        })
}

/// A startup identity without a transcript is a genuinely new session,
/// whatever sidecar is beside the absent transcript: a home there is an
/// orphan (a save that never committed a message, a transcript removed
/// by hand) and is discarded under the claim rather than admitted, so a
/// key with no history never refuses to start, and a stale home can never
/// be inherited by the first transcript written under the key.
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
