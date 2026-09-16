//! Atomic durability boundary for folder-aware resume decisions.
use crate::domain::{
    session::Session, session_identity::SessionIdentity, session_scope::SessionScopeMetadata,
};
use std::{future::Future, pin::Pin};

pub type ResumeTransactionResult<'a> =
    Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;

/// Infrastructure stages authoritative transcript and derived scope projection and
/// makes both visible at one commit point. Failure/cancellation must leave neither.
pub trait AtomicResumePersistence: Send + Sync {
    fn claim(&self, key: &SessionIdentity) -> Result<(), String>;
    fn release(&self, key: &SessionIdentity);
    fn commit<'a>(
        &'a self,
        session: &'a Session,
        scope: &'a SessionScopeMetadata,
    ) -> ResumeTransactionResult<'a>;
    fn rollback(&self, key: &SessionIdentity);
}

/// Affirmative launch capability. `Ready` means canonical target exists and a fresh
/// process can reload configuration/tools there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchCapability {
    Ready { canonical_directory: String },
    Unavailable { reason: String },
}
pub type ResumeEffect<'a, T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send + 'a>>;
pub trait ResumeDecisionEffects: Send + Sync {
    fn claim(&self, key: &SessionIdentity) -> Result<(), String>;
    fn release(&self, key: &SessionIdentity);
    fn load<'a>(&'a self, key: &'a SessionIdentity) -> ResumeEffect<'a, Session>;
    fn fresh_identity(&self) -> SessionIdentity;
    fn commit<'a>(
        &'a self,
        session: &'a Session,
        scope: &'a SessionScopeMetadata,
    ) -> ResumeEffect<'a, ()>;
    fn rollback(&self, key: &SessionIdentity);
    fn launch_fresh<'a>(
        &'a self,
        location: &'a str,
        key: &'a SessionIdentity,
    ) -> ResumeEffect<'a, ()>;
    fn publish_fork(&self, session: Session);
    fn publish_reassociation(&self, key: &SessionIdentity, scope: &SessionScopeMetadata);
}

pub trait SameScopeRestoreContext {
    fn restore<'a>(
        &'a mut self,
        key: &'a SessionIdentity,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;
}

pub trait ExactSessionAccess: Send + Sync {
    fn load<'a>(
        &'a self,
        key: &'a SessionIdentity,
    ) -> Pin<Box<dyn Future<Output = Result<Session, String>> + Send + 'a>>;
}
pub trait ResumeProjectionPublisher: Send + Sync {
    fn fork(&self, session: Session);
    fn reassociate(&self, key: &SessionIdentity, scope: &SessionScopeMetadata);
}

pub trait FreshRuntimeLaunch: Send + Sync {
    fn capability(&self, target: &str) -> LaunchCapability;
    fn launch<'a>(
        &'a self,
        canonical_directory: &'a str,
        key: &'a SessionIdentity,
    ) -> ResumeTransactionResult<'a>;
}
