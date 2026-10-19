//! The admission service's capability ports: trusted registration, the
//! authenticated client operations, the serialized dispatcher and the
//! owner-only recovery operations (moved from `services/` by #1910).

use crate::domain::admission::value_objects::inference_admission::{
    AbandonReport, AdmissionError, AdmissionLedger, Feedback, GroupId, GroupSnapshot, RequestId,
    RequestState, ScopeId, ThrottleFeedback, WorkloadClass,
};

/// Trusted authority ingress only; not exposed to an unauthenticated client.
pub trait AdmissionRegistry {
    fn register_root(&mut self, class: WorkloadClass) -> Result<ScopeId, AdmissionError>;
    fn register_child(&mut self, parent: ScopeId) -> Result<ScopeId, AdmissionError>;
    fn retire(&mut self, scope: ScopeId) -> Result<(), AdmissionError>;
}

/// Callers must supply their authenticated scope, never a caller-selected identity.
/// The future IPC adapter binds this input to a capability before invoking the port.
pub trait AdmissionClient {
    /// Apply receipt feedback once, retaining transport occupancy until completion.
    fn report_feedback(
        &mut self,
        scope: ScopeId,
        sequence: u64,
        report: u64,
        feedback: ThrottleFeedback,
        now: u64,
    ) -> Result<(), AdmissionError>;
    fn enqueue(
        &mut self,
        scope: ScopeId,
        sequence: u64,
        alias: &str,
        now: u64,
    ) -> Result<RequestState, AdmissionError>;
    fn cancel(
        &mut self,
        scope: ScopeId,
        sequence: u64,
        now: u64,
    ) -> Result<RequestState, AdmissionError>;
    fn complete(
        &mut self,
        scope: ScopeId,
        sequence: u64,
        feedback: Feedback,
        now: u64,
    ) -> Result<RequestState, AdmissionError>;
    fn status(
        &mut self,
        scope: ScopeId,
        sequence: u64,
        now: u64,
    ) -> Result<RequestState, AdmissionError>;
}

/// One serialized authority dispatches grants; clients do not run a scheduler.
pub trait AdmissionDispatcher {
    fn next(&mut self, group: &GroupId, now: u64) -> Result<Option<RequestId>, AdmissionError>;
    fn snapshot(&mut self, group: &GroupId, now: u64) -> Result<GroupSnapshot, AdmissionError>;
}

/// Owner-only recovery operations: abandonment on disconnect, grant withdrawal
/// when durability fails, scheduler wake computation, ledger export and the
/// operator epoch reset. Never exposed through a client capability.
pub trait AdmissionRecovery {
    fn high_water(&self, scope: ScopeId) -> Result<u64, AdmissionError>;
    fn abandon(&mut self, scope: ScopeId, now: u64) -> Result<AbandonReport, AdmissionError>;
    fn withdraw(&mut self, scope: ScopeId, sequence: u64, now: u64) -> Result<(), AdmissionError>;
    fn next_wake(&mut self, now: u64) -> Result<Option<u64>, AdmissionError>;
    fn ledger(&mut self, now: u64) -> Result<AdmissionLedger, AdmissionError>;
    fn reset(&mut self, now: u64) -> Result<u64, AdmissionError>;
}
