//! Production composition facade for resume decisions.
use super::{
    ports::{
        FreshSessionIdentityGenerator,
        resume_transaction::{
            AtomicResumePersistence, ExactSessionAccess, FreshRuntimeLaunch, LaunchCapability,
            ResumeProjectionPublisher,
        },
    },
    resume_decision::{ResumeDecisionEffects, ResumeEffect},
};
use crate::domain::{
    session::Session, session_identity::SessionIdentity, session_scope::SessionScopeMetadata,
};
use std::sync::Arc;
pub struct ProductionResumeDecisionFacade {
    access: Arc<dyn ExactSessionAccess>,
    persistence: Arc<dyn AtomicResumePersistence>,
    launcher: Arc<dyn FreshRuntimeLaunch>,
    publisher: Arc<dyn ResumeProjectionPublisher>,
    identities: Arc<dyn FreshSessionIdentityGenerator>,
}
impl ProductionResumeDecisionFacade {
    pub fn new(
        access: Arc<dyn ExactSessionAccess>,
        persistence: Arc<dyn AtomicResumePersistence>,
        launcher: Arc<dyn FreshRuntimeLaunch>,
        publisher: Arc<dyn ResumeProjectionPublisher>,
        identities: Arc<dyn FreshSessionIdentityGenerator>,
    ) -> Self {
        Self {
            access,
            persistence,
            launcher,
            publisher,
            identities,
        }
    }
}
impl ResumeDecisionEffects for ProductionResumeDecisionFacade {
    fn claim(&self, key: &SessionIdentity) -> Result<(), String> {
        self.persistence.claim(key)
    }
    fn release(&self, key: &SessionIdentity) {
        self.persistence.release(key)
    }
    fn load<'a>(&'a self, key: &'a SessionIdentity) -> ResumeEffect<'a, Session> {
        self.access.load(key)
    }
    fn fresh_identity(&self) -> SessionIdentity {
        self.identities.fresh_identity()
    }
    fn commit<'a>(
        &'a self,
        s: &'a Session,
        scope: &'a SessionScopeMetadata,
    ) -> ResumeEffect<'a, ()> {
        self.persistence.commit(s, scope)
    }
    fn rollback(&self, key: &SessionIdentity) {
        self.persistence.rollback(key)
    }
    fn launch_fresh<'a>(
        &'a self,
        location: &'a str,
        key: &'a SessionIdentity,
    ) -> ResumeEffect<'a, ()> {
        Box::pin(async move {
            match self.launcher.capability(location) {
                LaunchCapability::Ready {
                    canonical_directory,
                } => self.launcher.launch(&canonical_directory, key).await,
                LaunchCapability::Unavailable { reason } => Err(reason),
            }
        })
    }
    fn publish_fork(&self, s: Session) {
        self.publisher.fork(s)
    }
    fn publish_reassociation(&self, key: &SessionIdentity, scope: &SessionScopeMetadata) {
        self.publisher.reassociate(key, scope)
    }
}
#[cfg(test)]
#[path = "production_resume_tests.rs"]
mod tests;
