//! The eligibility collaborators of the resume transaction (#2011). Not a
//! second owner: `admit` routes a request's intent for the one transaction in
//! the parent, and `decide` is the one eligibility check its effect-free
//! pre-flight and its claimed re-check both run.
use super::ResumeSavedSession;
use crate::application::sessions::dto::{
    ActionAvailability, ResumeActionCapabilities, ResumeActionOffer, ResumeDecision, ResumeIntent,
    ResumeRequest, ResumeSavedSessionError, ResumeTarget,
};
use crate::application::sessions::ports::SessionStore;
use crate::application::sessions::session_home::{HomeObstacle, SessionHomeContext};
use crate::domain::resume_decision::{HomeVersion, ResumeAction};
use crate::domain::session::Session;
use crate::domain::session_home::SessionHomeScope;

/// What the admission of a request leaves to do.
pub(super) enum Admitted {
    /// The client cancelled: nothing is settled, saved, claimed or restored.
    Cancelled(String),
    /// The one restore transaction, for this exact target.
    Restore(ResumeTarget),
}

/// Admit a request before any effect. An ephemeral loop resumes nothing; the
/// target is one of the accepted exact spellings — no prefix, no fuzzy
/// match. `Cancel` has no effect at all. Every other explicit action is
/// refused here — with the reason its executor is not composed, or because
/// it has its own transaction — and is never replaced by a restore or by
/// another action.
pub(super) fn admit(
    ephemeral: bool,
    capabilities: &ResumeActionCapabilities,
    request: &ResumeRequest,
) -> Result<Admitted, ResumeSavedSessionError> {
    if ephemeral {
        return Err(ResumeSavedSessionError::Ephemeral);
    }
    let target = ResumeTarget::parse(&request.target)?;
    match request.intent {
        ResumeIntent::Restore => Ok(Admitted::Restore(target)),
        ResumeIntent::Act(ResumeAction::Cancel) => Ok(Admitted::Cancelled(target.name)),
        ResumeIntent::Act(action) => Err(match capabilities.availability(action) {
            ActionAvailability::Unavailable(reason) => {
                ResumeSavedSessionError::ActionUnavailable { action, reason }
            }
            ActionAvailability::Available => {
                ResumeSavedSessionError::ActionExecutedElsewhere(action)
            }
        }),
    }
}

/// The eligibility collaborators of one loop: the shared home observations
/// and the explicit actions composition declared executable.
pub(super) struct Eligibility {
    pub(super) home: SessionHomeContext,
    pub(super) capabilities: ResumeActionCapabilities,
}

impl Eligibility {
    /// Decide without any effect: a decision or a stale selection of a
    /// target the store can read then costs no settlement, save or claim.
    /// An absent or unreadable target decides nothing here — the claimed
    /// path reports it.
    pub(super) async fn preflight(
        &self,
        store: &dyn SessionStore,
        target: &ResumeTarget,
        expected: Option<&HomeVersion>,
    ) -> Result<(), ResumeSavedSessionError> {
        let Err(obstacle) = decide(&self.home, &self.capabilities, target, expected).await else {
            return Ok(());
        };
        match store.load(&target.identity).await {
            Ok(Some(_)) => Err(obstacle),
            Ok(None) | Err(_) => Ok(()),
        }
    }

    /// Read the claimed target and re-decide under the claim; on `Err` the
    /// caller's guard releases a claim not the loop's own.
    pub(super) async fn load_claimed(
        &self,
        store: &dyn SessionStore,
        target: &ResumeTarget,
        expected: Option<&HomeVersion>,
    ) -> Result<Session, ResumeSavedSessionError> {
        match store.load(&target.identity).await {
            Ok(Some(session)) => {
                decide(&self.home, &self.capabilities, target, expected).await?;
                Ok(session)
            }
            Ok(None) => Err(ResumeSavedSessionError::NotFound(target.name.clone())),
            Err(err) => Err(ResumeSavedSessionError::Load(err)),
        }
    }
}

/// The one eligibility check: the authoritative home is read fresh (never the
/// derived index), the version the client was shown must still be it, and
/// only an affirmatively admitted home passes. Everything else is a typed
/// decision or refusal.
async fn decide(
    home: &SessionHomeContext,
    capabilities: &ResumeActionCapabilities,
    target: &ResumeTarget,
    expected: Option<&HomeVersion>,
) -> Result<(), ResumeSavedSessionError> {
    let scope = home
        .catalogue
        .read(&target.identity)
        .map_err(ResumeSavedSessionError::Load)?;
    let home_version = HomeVersion::of(&scope);
    if expected.is_some_and(|expected| *expected != home_version) {
        return Err(ResumeSavedSessionError::StaleHomeVersion);
    }
    let (kind, detail) = match home.classify(&scope).await {
        Ok(()) => return Ok(()),
        Err(HomeObstacle::CurrentUnavailable(reason)) => {
            return Err(ResumeSavedSessionError::CurrentScopeUnavailable(reason));
        }
        Err(HomeObstacle::Decision(kind, detail)) => (kind, detail),
    };
    let offers = kind
        .offered_actions()
        .iter()
        .map(|action| ResumeActionOffer {
            action: *action,
            availability: capabilities.availability(*action),
        })
        .collect();
    let execution_dir = match scope {
        SessionHomeScope::Scoped(saved) => Some(saved.execution_dir),
        SessionHomeScope::LegacyUnscoped | SessionHomeScope::Unavailable(_) => None,
    };
    Err(ResumeSavedSessionError::Decision(Box::new(
        ResumeDecision {
            target: target.clone(),
            kind,
            home_version,
            execution_dir,
            detail,
            offers,
        },
    )))
}

impl std::fmt::Debug for ResumeSavedSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResumeSavedSession")
            .field("ephemeral", &self.ephemeral)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "resume_saved_session_decision_tests.rs"]
mod tests;
