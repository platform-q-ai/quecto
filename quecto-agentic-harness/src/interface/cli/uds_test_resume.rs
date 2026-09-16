#![cfg(test)]
use crate::application::sessions::ports::resume_transaction::{
    ResumeDecisionEffects, ResumeEffect,
};
use crate::domain::{
    session::Session, session_identity::SessionIdentity, session_scope::SessionScopeMetadata,
};
use std::sync::Arc;
struct Stub;
impl ResumeDecisionEffects for Stub {
    fn claim(&self, _: &SessionIdentity) -> Result<(), String> {
        Ok(())
    }
    fn release(&self, _: &SessionIdentity) {}
    fn load<'a>(&'a self, _: &'a SessionIdentity) -> ResumeEffect<'a, Session> {
        Box::pin(async { Err("unused test resume".into()) })
    }
    fn fresh_identity(&self) -> SessionIdentity {
        panic!("unused test resume identity")
    }
    fn commit<'a>(&'a self, _: &'a Session, _: &'a SessionScopeMetadata) -> ResumeEffect<'a, ()> {
        Box::pin(async { Ok(()) })
    }
    fn rollback(&self, _: &SessionIdentity) {}
    fn launch_fresh<'a>(&'a self, _: &'a str, _: &'a SessionIdentity) -> ResumeEffect<'a, ()> {
        Box::pin(async { Ok(()) })
    }
    fn publish_fork(&self, _: Session) {}
    fn publish_reassociation(&self, _: &SessionIdentity, _: &SessionScopeMetadata) {}
}
pub(crate) fn handle() -> Arc<dyn ResumeDecisionEffects> {
    Arc::new(Stub)
}
