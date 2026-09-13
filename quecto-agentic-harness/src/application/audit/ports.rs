//! Capability-local ports for the audit capability.
use std::future::Future;
use std::pin::Pin;

use crate::domain::audit::AuditEvent;
use crate::domain::error::DomainError;

/// Port: an audit event sink. Implemented by `AuditLog` in infrastructure;
/// the agent loop emits through it without importing infrastructure types.
///
/// Uses boxed futures instead of `async fn` for dyn-compatibility.
pub trait AuditSink: Send + Sync {
    fn emit(
        &self,
        turn: u32,
        event: AuditEvent,
    ) -> Pin<Box<dyn Future<Output = Result<(), DomainError>> + Send + '_>>;
}
