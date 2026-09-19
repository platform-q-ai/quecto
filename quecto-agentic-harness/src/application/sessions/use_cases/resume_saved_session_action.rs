//! The answer to an explicit action other than `cancel` (#2011): effect-free,
//! in one order — the target exists, it names the version the client was
//! shown, that version is still the authority's, the decision kind offers the
//! action, its executor is composed. Nothing is settled, saved or claimed, and
//! an action is never replaced by a restore or by another action. The
//! executors of #2012–#2014 route their action before this owner and inherit
//! the order: whatever they do not re-check themselves is not checked.
use super::resume_saved_session_decision::{Eligibility, decide};
use crate::application::sessions::dto::{
    ActionAvailability, ResumeSavedSessionError, ResumeTarget,
};
use crate::application::sessions::ports::SessionStore;
use crate::domain::resume_decision::{HomeVersion, ResumeAction};

impl Eligibility {
    /// Why `action` is not executed by the restore owner. The loop's own key
    /// has no exception here: an action saves nothing, so an absent record
    /// stays absent; a store that cannot answer is a load failure.
    pub(super) async fn refuse(
        &self,
        store: &dyn SessionStore,
        target: &ResumeTarget,
        expected: Option<&HomeVersion>,
        action: ResumeAction,
    ) -> ResumeSavedSessionError {
        match SessionStore::exists(store, &target.identity).await {
            Ok(true) => {}
            Ok(false) => return ResumeSavedSessionError::NotFound(target.name.clone()),
            Err(err) => return ResumeSavedSessionError::Load(err),
        }
        if expected.is_none() {
            return ResumeSavedSessionError::HomeVersionRequired(action);
        }
        let offered = match decide(&self.home, &self.capabilities, target, expected).await {
            // The home admits a plain restore: no decision, so no offer.
            Ok(()) => false,
            Err(ResumeSavedSessionError::Decision(decision)) => decision.offer(action).is_some(),
            Err(refusal) => return refusal,
        };
        if !offered {
            return ResumeSavedSessionError::ActionNotOffered(action);
        }
        match self.capabilities.availability(action) {
            ActionAvailability::Unavailable(reason) => {
                ResumeSavedSessionError::ActionUnavailable { action, reason }
            }
            ActionAvailability::Available => {
                ResumeSavedSessionError::ActionExecutedElsewhere(action)
            }
        }
    }
}
