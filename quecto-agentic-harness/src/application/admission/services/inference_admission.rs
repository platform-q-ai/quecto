//! The serialized in-memory admission service: it implements the admission
//! ports (`ports::inference_admission`) over the domain policy, and the
//! authority (`inference_authority`) owns it in the running broker.
use crate::application::admission::ports::inference_admission::{
    AdmissionClient, AdmissionDispatcher, AdmissionRecovery, AdmissionRegistry,
};
use crate::domain::admission::value_objects::inference_admission::*;

/// Serialized in-memory service: the authority owns one in the running
/// broker, and the contract suites drive it directly.
#[derive(Debug)]
pub struct AdmissionService {
    policy: crate::domain::admission::services::inference_admission_policy::AdmissionPolicy,
}

impl AdmissionService {
    pub fn new(epoch: u64, config: AdmissionConfig) -> Result<Self, AdmissionError> {
        Ok(Self {
            policy: crate::domain::admission::services::inference_admission_policy::AdmissionPolicy::new(epoch, config)?,
        })
    }

    /// Rehydrate from a durable ledger: same epoch, outstanding work orphaned.
    pub fn restore(
        config: AdmissionConfig,
        ledger: &AdmissionLedger,
        now: u64,
    ) -> Result<Self, AdmissionError> {
        Ok(Self {
            policy: crate::domain::admission::services::inference_admission_policy::AdmissionPolicy::restore(
                config, ledger, now,
            )?,
        })
    }
}

impl AdmissionRecovery for AdmissionService {
    fn high_water(&self, scope: ScopeId) -> Result<u64, AdmissionError> {
        self.policy.high_water(scope)
    }
    fn abandon(&mut self, scope: ScopeId, now: u64) -> Result<AbandonReport, AdmissionError> {
        self.policy.abandon(scope, now)
    }
    fn withdraw(&mut self, scope: ScopeId, sequence: u64, now: u64) -> Result<(), AdmissionError> {
        self.policy.withdraw(scope, sequence, now)
    }
    fn next_wake(&mut self, now: u64) -> Result<Option<u64>, AdmissionError> {
        self.policy.next_wake(now)
    }
    fn ledger(&mut self, now: u64) -> Result<AdmissionLedger, AdmissionError> {
        self.policy.ledger(now)
    }
    fn reset(&mut self, now: u64) -> Result<u64, AdmissionError> {
        self.policy.reset(now)
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
    fn report_feedback(
        &mut self,
        scope: ScopeId,
        sequence: u64,
        report: u64,
        feedback: ThrottleFeedback,
        now: u64,
    ) -> Result<(), AdmissionError> {
        self.policy
            .report_feedback(scope, sequence, report, feedback, now)
    }
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
