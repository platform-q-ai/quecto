//! The typed entry and the eligibility collaborators of the resume
//! transaction (#2011). Not a second owner: `request` routes an intent to the
//! one restore in the parent, and `decide` is the one eligibility check the
//! effect-free pre-flight and the claimed re-check both run.
use super::ResumeSavedSession;
use crate::application::sessions::dto::{
    ActionAvailability, ResumeActionCapabilities, ResumeActionOffer, ResumeDecision, ResumeIntent,
    ResumeOutcome, ResumeRequest, ResumeSavedSessionError, ResumeTarget,
};
use crate::application::sessions::ports::{FleetSettlement, SessionSwitchRuntime};
use crate::application::sessions::session_home::{HomeObstacle, SessionHomeContext};
use crate::domain::message::Message;
use crate::domain::resume_decision::{HomeVersion, ResumeAction};
use crate::domain::session::Session;
use crate::domain::session_home::SessionHomeScope;

impl ResumeSavedSession {
    /// Declare the explicit actions whose executor composition wired.
    #[must_use]
    pub fn with_capabilities(mut self, capabilities: ResumeActionCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    /// Answer a typed resume request. `Cancel` has no effect at all. Every
    /// other explicit action is refused before any effect — with the reason
    /// its executor is not composed, or because it has its own transaction —
    /// and is never replaced by a restore or by another action. `Restore`
    /// runs the one restore transaction.
    pub async fn request(
        &self,
        request: &ResumeRequest,
        messages: &mut Vec<Message>,
        fleet: Option<&dyn FleetSettlement>,
        runtime: &mut dyn SessionSwitchRuntime,
    ) -> Result<ResumeOutcome, ResumeSavedSessionError> {
        let target = self.admit_target(&request.target)?;
        match request.intent {
            ResumeIntent::Act(ResumeAction::Cancel) => {
                Ok(ResumeOutcome::Cancelled { name: target.name })
            }
            ResumeIntent::Act(action) => Err(self.refuse_action(action)),
            ResumeIntent::Restore => {
                let expected = request.expected_home_version.as_ref();
                self.restore(target, expected, messages, fleet, runtime)
                    .await
                    .map(ResumeOutcome::Resumed)
            }
        }
    }

    /// An ephemeral loop resumes nothing; the target is one of the accepted
    /// exact spellings — no prefix, no fuzzy match.
    pub(super) fn admit_target(&self, raw: &str) -> Result<ResumeTarget, ResumeSavedSessionError> {
        if self.ephemeral {
            return Err(ResumeSavedSessionError::Ephemeral);
        }
        ResumeTarget::parse(raw)
    }

    fn refuse_action(&self, action: ResumeAction) -> ResumeSavedSessionError {
        match self.capabilities.availability(action) {
            ActionAvailability::Unavailable(reason) => {
                ResumeSavedSessionError::ActionUnavailable { action, reason }
            }
            ActionAvailability::Available => {
                ResumeSavedSessionError::ActionExecutedElsewhere(action)
            }
        }
    }

    /// Decide without any effect, when the store affirms the target exists:
    /// a decision or a stale selection then costs no settlement, save or
    /// claim. An absent or unobservable target decides nothing here — the
    /// claimed path reports it.
    pub(super) async fn preflight(
        &self,
        target: &ResumeTarget,
        expected: Option<&HomeVersion>,
    ) -> Result<(), ResumeSavedSessionError> {
        match self.store.exists(&target.identity).await {
            Ok(true) => decide(&self.home, &self.capabilities, target, expected).await,
            Ok(false) | Err(_) => Ok(()),
        }
    }

    /// Read the claimed target and re-decide under the claim; on `Err` the
    /// guard releases a claim not the loop's own.
    pub(super) async fn load_claimed(
        &self,
        target: &ResumeTarget,
        expected: Option<&HomeVersion>,
    ) -> Result<Session, ResumeSavedSessionError> {
        match self.store.load(&target.identity).await {
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
