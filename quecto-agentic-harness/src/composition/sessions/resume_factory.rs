use super::build_fresh_session_identity;
use crate::application::sessions::ports::SessionStore;
use crate::application::sessions::{
    ports::resume_transaction::{
        ExactSessionAccess, ResumeDecisionEffects, ResumeEffect, ResumeProjectionPublisher,
    },
    production_resume::ProductionResumeDecisionFacade,
};
use crate::domain::{
    session::Session, session_identity::SessionIdentity, session_scope::SessionScopeMetadata,
};
use crate::infrastructure::persistence::session_layout::FlatSessionLayout;
use std::sync::Arc;
struct StoreAccess(Arc<dyn SessionStore>);
impl ExactSessionAccess for StoreAccess {
    fn load<'a>(&'a self, key: &'a SessionIdentity) -> ResumeEffect<'a, Session> {
        Box::pin(async move {
            self.0
                .load(key)
                .await
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("session not found: {}", key.runtime_key()))
        })
    }
}
struct DerivedPublisher;
impl ResumeProjectionPublisher for DerivedPublisher {
    fn fork(&self, _: Session) {}
    fn reassociate(&self, _: &SessionIdentity, _: &SessionScopeMetadata) {}
}
pub fn production_resume_handle(
    base: &std::path::Path,
    store: Arc<dyn SessionStore>,
) -> Result<Arc<dyn ResumeDecisionEffects>, String> {
    let launcher = Arc::new(
        crate::infrastructure::resume_decision_adapter::FreshRuntimeLauncher::current_executable()?,
    );
    Ok(Arc::new(ProductionResumeDecisionFacade::new(
        Arc::new(StoreAccess(store)),
        Arc::new(
            crate::infrastructure::resume_decision_adapter::FileSessionScopeTransaction::new(
                FlatSessionLayout::new(base),
            ),
        ),
        launcher,
        Arc::new(DerivedPublisher),
        build_fresh_session_identity(),
    )))
}
