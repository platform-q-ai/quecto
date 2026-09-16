//! Contract inventory for folder-aware resume capability ports.
use quecto::application::sessions::{
    ports::resume_transaction::SameScopeRestoreContext,
    ports::{
        resume_transaction::{
            AtomicResumePersistence, ExactSessionAccess, FreshRuntimeLaunch,
            ResumeProjectionPublisher,
        },
        scope_discovery::SessionScopeDiscovery,
    },
    resume_decision::ResumeDecisionEffects,
};

#[test]
fn public_resume_ports_are_object_safe_at_the_composition_boundary() {
    fn persistence(_: Option<&dyn AtomicResumePersistence>) {}
    fn access(_: Option<&dyn ExactSessionAccess>) {}
    fn launcher(_: Option<&dyn FreshRuntimeLaunch>) {}
    fn publisher(_: Option<&dyn ResumeProjectionPublisher>) {}
    fn effects(_: Option<&dyn ResumeDecisionEffects>) {}
    fn restore(_: Option<&dyn SameScopeRestoreContext>) {}
    fn discovery(_: Option<&dyn SessionScopeDiscovery>) {}
    persistence(None);
    access(None);
    launcher(None);
    publisher(None);
    effects(None);
    restore(None);
    discovery(None);
}
