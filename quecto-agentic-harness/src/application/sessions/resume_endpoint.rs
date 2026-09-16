//! Application-owned endpoint for explicit folder-aware resume decisions.

use crate::domain::session_identity::SessionIdentity;
use crate::domain::session_scope::{
    AssociationProvenance, CanonicalExecutionLocation, SessionHomeScope, SessionScopeMetadata,
};

use super::ports::resume_transaction::SameScopeRestoreContext;
use super::resume_decision::{
    ResumeDecisionEffects, ResumeOutcome, ResumePlan, execute_resume_decision,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeEndpointAction {
    OpenOriginal,
    ForkCurrent,
    Locate,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeEndpointRequest {
    pub persisted_key: String,
    pub action: ResumeEndpointAction,
    pub location: Option<String>,
}

pub async fn execute_resume_endpoint(
    effects: &dyn ResumeDecisionEffects,
    request: ResumeEndpointRequest,
) -> Result<ResumeOutcome, String> {
    let eligible = match request.action {
        ResumeEndpointAction::Cancel => true,
        ResumeEndpointAction::OpenOriginal | ResumeEndpointAction::Locate => {
            !request.persisted_key.trim().is_empty()
                && request
                    .location
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
        }
        ResumeEndpointAction::ForkCurrent => !request.persisted_key.trim().is_empty(),
    };
    if !eligible {
        return Err(
            "resume decision is not eligible: required session/location unavailable".into(),
        );
    }

    let key = super::use_cases::read_history::exact_persisted_identity(request.persisted_key);
    let source = effects.load(&key).await?;
    let plan = match request.action {
        ResumeEndpointAction::OpenOriginal => ResumePlan::LaunchOriginal,
        ResumeEndpointAction::ForkCurrent => ResumePlan::ForkTranscriptOnly,
        ResumeEndpointAction::Locate => ResumePlan::LocateAndReassociate,
        ResumeEndpointAction::Cancel => ResumePlan::Cancel,
    };
    let home = match request.location {
        Some(value) => SessionHomeScope::scoped(
            CanonicalExecutionLocation::new(value).map_err(str::to_owned)?,
            None,
            AssociationProvenance::ExplicitlyLocated,
        ),
        None => SessionHomeScope::LegacyUnscoped,
    };
    let fresh = matches!(plan, ResumePlan::ForkTranscriptOnly).then(|| effects.fresh_identity());
    struct Refuse;
    impl SameScopeRestoreContext for Refuse {
        fn restore<'a>(
            &'a mut self,
            _: &'a SessionIdentity,
        ) -> super::resume_decision::ResumeEffect<'a, ()> {
            Box::pin(async { Err("same-scope restore unavailable".into()) })
        }
    }
    execute_resume_decision(
        effects,
        plan,
        &source,
        &SessionScopeMetadata::current(home),
        fresh,
        &mut Refuse,
    )
    .await
}

#[cfg(test)]
#[path = "resume_endpoint_tests.rs"]
mod tests;
