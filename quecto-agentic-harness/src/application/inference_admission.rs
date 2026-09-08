//! Phase-local admission application capabilities. No runtime wiring is enabled.
use crate::domain::inference_admission::*;

/// Trusted authority ingress only; not exposed to an unauthenticated client.
pub trait AdmissionRegistry {
    fn register_root(&mut self, class: WorkloadClass) -> Result<ScopeId, AdmissionError>;
    fn register_child(&mut self, parent: ScopeId) -> Result<ScopeId, AdmissionError>;
    fn retire(&mut self, scope: ScopeId) -> Result<(), AdmissionError>;
}

/// Callers must supply their authenticated scope, never a caller-selected identity.
/// The future IPC adapter binds this input to a capability before invoking the port.
pub trait AdmissionClient {
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

/// Serialized in-memory service for contracts and future broker composition.
/// Not installed at provider boundaries until the later integration phases.
#[derive(Debug)]
pub struct AdmissionService {
    policy: crate::domain::inference_admission_policy::AdmissionPolicy,
}

impl AdmissionService {
    pub fn new(epoch: u64, config: AdmissionConfig) -> Result<Self, AdmissionError> {
        Ok(Self {
            policy: crate::domain::inference_admission_policy::AdmissionPolicy::new(epoch, config)?,
        })
    }
}

impl AdmissionRegistry for AdmissionService {
    fn register_root(&mut self, class: WorkloadClass) -> Result<ScopeId, AdmissionError> {
        self.policy.register_root(class)
    }
    fn register_child(&mut self, parent: ScopeId) -> Result<ScopeId, AdmissionError> {
        self.policy.register_child(parent)
    }
    fn retire(&mut self, scope: ScopeId) -> Result<(), AdmissionError> {
        self.policy.retire(scope)
    }
}
impl AdmissionClient for AdmissionService {
    fn enqueue(
        &mut self,
        scope: ScopeId,
        sequence: u64,
        alias: &str,
        now: u64,
    ) -> Result<RequestState, AdmissionError> {
        self.policy.enqueue(scope, sequence, alias, now)
    }
    fn cancel(
        &mut self,
        scope: ScopeId,
        sequence: u64,
        now: u64,
    ) -> Result<RequestState, AdmissionError> {
        self.policy.cancel(scope, sequence, now)
    }
    fn complete(
        &mut self,
        scope: ScopeId,
        sequence: u64,
        feedback: Feedback,
        now: u64,
    ) -> Result<RequestState, AdmissionError> {
        self.policy.complete(scope, sequence, feedback, now)
    }
    fn status(
        &mut self,
        scope: ScopeId,
        sequence: u64,
        now: u64,
    ) -> Result<RequestState, AdmissionError> {
        self.policy.status(scope, sequence, now)
    }
}
impl AdmissionDispatcher for AdmissionService {
    fn next(&mut self, group: &GroupId, now: u64) -> Result<Option<RequestId>, AdmissionError> {
        self.policy.next(group, now)
    }
    fn snapshot(&mut self, group: &GroupId, now: u64) -> Result<GroupSnapshot, AdmissionError> {
        self.policy.snapshot(group, now)
    }
}
