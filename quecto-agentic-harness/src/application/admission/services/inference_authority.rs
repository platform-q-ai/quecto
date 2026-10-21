//! Authenticated, durability-gated admission authority over the P1 service.
//!
//! Every client operation carries a capability issued here. Grants become
//! visible only after the ledger is durable; completions are acknowledged only
//! after the release is durable. Failure of either path fails closed.
//!
//! The durability sequences (dispatch, completion, epoch reset, checkpoint)
//! and the running broker's session policy (settling a turn, binding and
//! disconnecting a connection's scope) are the admission use cases' (#1910):
//! they hold the journal port and drive the raw operations here, which are
//! visible only inside the capability. The authority is the single owner of
//! the journal-health flag; only those use cases set it.
use std::collections::{BTreeMap, BTreeSet};

use crate::application::admission::dto::broker::{AuthorityError, AuthorityStatus, Credential};
use crate::application::admission::ports::inference_admission::{
    AdmissionClient, AdmissionDispatcher, AdmissionRecovery, AdmissionRegistry,
};
use crate::application::admission::ports::inference_authority_ports::{
    AdmissionSecretSource, JournalError,
};
use crate::application::admission::services::inference_admission::AdmissionService;
use crate::domain::admission::value_objects::inference_admission::*;
use crate::domain::shared::secret_compare::constant_time_eq;

/// The journal failing is the authority's `JournalUnavailable`: grants and
/// acknowledgements fail closed.
impl From<JournalError> for AuthorityError {
    fn from(_: JournalError) -> Self {
        Self::JournalUnavailable
    }
}

#[derive(Debug)]
struct Issued {
    token: String,
    root: ScopeId,
    parent: Option<ScopeId>,
}

#[derive(Debug)]
pub struct AdmissionAuthority<S> {
    service: AdmissionService,
    secrets: S,
    issued: BTreeMap<ScopeId, Issued>,
    /// Scopes whose connection ended while they still held active work, and
    /// that no connection has bound since: once that work leaves (its lease
    /// clears, #2664) nothing can use the capability, so it is released.
    detached: BTreeSet<ScopeId>,
    groups: Vec<GroupId>,
    journal_healthy: bool,
    /// Outstanding attempts in the last durable ledger: occupancy that left
    /// without a write since (a cleared lease, #2664) is checkpointed.
    durable_outstanding: usize,
}

impl<S: AdmissionSecretSource> AdmissionAuthority<S> {
    pub fn new(epoch: u64, config: AdmissionConfig, secrets: S) -> Result<Self, AuthorityError> {
        let groups = config.groups.keys().cloned().collect();
        Ok(Self::compose(
            AdmissionService::new(epoch, config)?,
            secrets,
            groups,
        ))
    }

    /// Rehydrate at the ledger's epoch with outstanding work orphaned.
    pub fn restore(
        config: AdmissionConfig,
        ledger: &AdmissionLedger,
        secrets: S,
        now: u64,
    ) -> Result<Self, AuthorityError> {
        let groups = config.groups.keys().cloned().collect();
        Ok(Self::compose(
            AdmissionService::restore(config, ledger, now)?,
            secrets,
            groups,
        ))
    }

    fn compose(service: AdmissionService, secrets: S, groups: Vec<GroupId>) -> Self {
        Self {
            service,
            secrets,
            issued: BTreeMap::new(),
            detached: BTreeSet::new(),
            groups,
            journal_healthy: true,
            durable_outstanding: 0,
        }
    }

    fn authenticate(&self, credential: &Credential) -> Result<(), AuthorityError> {
        match self.issued.get(&credential.scope) {
            Some(issued) if constant_time_eq(&issued.token, &credential.token) => Ok(()),
            _ => Err(AuthorityError::Unauthorized),
        }
    }

    fn issue(&mut self, scope: ScopeId, root: ScopeId, parent: Option<ScopeId>) -> Credential {
        let token = self.secrets.mint();
        debug_assert!(!token.is_empty(), "secret source must mint material");
        self.issued.insert(
            scope,
            Issued {
                token: token.clone(),
                root,
                parent,
            },
        );
        Credential { scope, token }
    }

    pub(in crate::application::admission) fn journal_healthy(&self) -> bool {
        self.journal_healthy
    }

    /// The journal's last outcome, recorded by the use case that wrote it.
    pub(in crate::application::admission) fn record_journal(&mut self, healthy: bool) {
        self.journal_healthy = healthy;
    }

    /// The ledger the journal accepted: how many attempts it holds.
    pub(in crate::application::admission) fn record_durable(&mut self, ledger: &AdmissionLedger) {
        self.durable_outstanding = ledger.outstanding.len();
    }

    /// Whether occupancy left since the last durable ledger without a write
    /// of its own (#2664: a lease or orphan cleared on time), so the journal
    /// must not bring it back after a restart.
    pub(in crate::application::admission) fn checkpoint_due(
        &mut self,
        now: u64,
    ) -> Result<bool, AuthorityError> {
        Ok(self.service.ledger(now)?.outstanding.len() < self.durable_outstanding)
    }

    /// The current ledger, for a use case to make durable.
    pub(in crate::application::admission) fn ledger(
        &mut self,
        now: u64,
    ) -> Result<AdmissionLedger, AuthorityError> {
        Ok(self.service.ledger(now)?)
    }

    pub fn register_root(
        &mut self,
        class: WorkloadClass,
        now: u64,
    ) -> Result<Credential, AuthorityError> {
        let _ = now;
        let scope = self.service.register_root(class)?;
        Ok(self.issue(scope, scope, None))
    }

    /// Children are registered by an authenticated parent before launch and are
    /// capped at background priority by the policy.
    pub fn register_child(
        &mut self,
        parent: &Credential,
        now: u64,
    ) -> Result<Credential, AuthorityError> {
        let _ = now;
        self.authenticate(parent)?;
        let root = self.issued[&parent.scope].root;
        let scope = self.service.register_child(parent.scope)?;
        Ok(self.issue(scope, root, Some(parent.scope)))
    }

    /// Prove a capability without acting on it; returns the scope's acquire
    /// high-water mark so a reconnecting client continues its sequence.
    pub(in crate::application::admission) fn verify(
        &self,
        credential: &Credential,
    ) -> Result<u64, AuthorityError> {
        self.authenticate(credential)?;
        Ok(self.service.high_water(credential.scope)?)
    }

    /// Owner-side observation of any request; never exposed to clients.
    pub fn observe(&mut self, id: RequestId, now: u64) -> Result<RequestState, AuthorityError> {
        Ok(self.service.status(id.scope, id.sequence, now)?)
    }

    pub fn lineage(&self, scope: ScopeId) -> Result<ScopeId, AuthorityError> {
        self.issued
            .get(&scope)
            .map(|issued| issued.root)
            .ok_or(AuthorityError::Unauthorized)
    }

    pub fn acquire(
        &mut self,
        credential: &Credential,
        sequence: u64,
        alias: &str,
        now: u64,
    ) -> Result<RequestState, AuthorityError> {
        self.authenticate(credential)?;
        Ok(self
            .service
            .enqueue(credential.scope, sequence, alias, now)?)
    }

    /// Select every grant the policy allows, not yet durable; the dispatch
    /// use case makes them durable or withdraws them. The first policy error
    /// is returned beside whatever was selected before it.
    pub(in crate::application::admission) fn select_grants(
        &mut self,
        now: u64,
    ) -> (Vec<RequestId>, Option<AdmissionError>) {
        let mut grants = Vec::new();
        let mut first_error = None;
        for group in self.groups.clone() {
            loop {
                match self.service.next(&group, now) {
                    Ok(Some(id)) => grants.push(id),
                    Ok(None) => break,
                    Err(error) => {
                        first_error.get_or_insert(error);
                        break;
                    }
                }
            }
        }
        (grants, first_error)
    }

    /// Withdraw grants that could not be made durable.
    pub(in crate::application::admission) fn withdraw_grants(
        &mut self,
        grants: &[RequestId],
        now: u64,
    ) -> Result<(), AuthorityError> {
        for id in grants {
            self.service.withdraw(id.scope, id.sequence, now)?;
        }
        Ok(())
    }

    pub fn cancel(
        &mut self,
        credential: &Credential,
        sequence: u64,
        now: u64,
    ) -> Result<RequestState, AuthorityError> {
        self.authenticate(credential)?;
        Ok(self.service.cancel(credential.scope, sequence, now)?)
    }

    pub fn feedback(
        &mut self,
        credential: &Credential,
        sequence: u64,
        report: u64,
        feedback: ThrottleFeedback,
        now: u64,
    ) -> Result<(), AuthorityError> {
        self.authenticate(credential)?;
        Ok(self
            .service
            .report_feedback(credential.scope, sequence, report, feedback, now)?)
    }

    /// Release a finished attempt in memory; the completion use case makes it
    /// durable before it is acknowledged.
    pub(in crate::application::admission) fn complete_unpersisted(
        &mut self,
        credential: &Credential,
        sequence: u64,
        feedback: Feedback,
        now: u64,
    ) -> Result<RequestState, AuthorityError> {
        self.authenticate(credential)?;
        Ok(self
            .service
            .complete(credential.scope, sequence, feedback, now)?)
    }

    pub fn status(
        &mut self,
        credential: &Credential,
        sequence: u64,
        now: u64,
    ) -> Result<RequestState, AuthorityError> {
        self.authenticate(credential)?;
        Ok(self.service.status(credential.scope, sequence, now)?)
    }

    pub fn retire(&mut self, credential: &Credential, now: u64) -> Result<(), AuthorityError> {
        let _ = now;
        self.authenticate(credential)?;
        self.service.retire(credential.scope)?;
        self.issued.remove(&credential.scope);
        self.detached.remove(&credential.scope);
        Ok(())
    }

    /// Owner-side release of a scope with no unverified work (its session
    /// closed cleanly). Refused as `Busy` while any attempt is still active.
    pub(in crate::application::admission) fn release(
        &mut self,
        scope: ScopeId,
    ) -> Result<(), AuthorityError> {
        self.service.retire(scope)?;
        self.issued.remove(&scope);
        self.detached.remove(&scope);
        Ok(())
    }

    /// A connection bound `scope`'s capability: it is live again, so its
    /// cleared work never releases it.
    pub(in crate::application::admission) fn attach(&mut self, scope: ScopeId) {
        self.detached.remove(&scope);
    }

    /// Release every detached scope that no longer holds active work (#2664)
    /// and answer how many were released. A scope still holding work stays;
    /// one the policy no longer knows is forgotten.
    pub(in crate::application::admission) fn release_idle_detached(&mut self) -> usize {
        let mut released = 0;
        for scope in std::mem::take(&mut self.detached) {
            match self.service.retire(scope) {
                Ok(()) => {
                    self.issued.remove(&scope);
                    released += 1;
                }
                Err(AdmissionError::Busy) => {
                    self.detached.insert(scope);
                }
                Err(AdmissionError::UnknownScope | AdmissionError::StaleEpoch) => {
                    self.issued.remove(&scope);
                }
                Err(other) => {
                    debug_assert!(false, "retire refuses only busy or unknown: {other:?}");
                    self.detached.insert(scope);
                }
            }
        }
        released
    }

    /// The uncertain leases the expired wait `id` was held by (#2664).
    pub(in crate::application::admission) fn held(
        &mut self,
        id: RequestId,
        now: u64,
    ) -> Result<Option<UncertainHold>, AuthorityError> {
        Ok(self.service.held(id.scope, id.sequence, now)?)
    }

    /// A parent retires a descendant it registered (for example a launch that
    /// never started). Only the direct parent may do so.
    pub fn retire_child(
        &mut self,
        parent: &Credential,
        child: ScopeId,
        now: u64,
    ) -> Result<(), AuthorityError> {
        let _ = now;
        self.authenticate(parent)?;
        match self.issued.get(&child) {
            Some(issued) if issued.parent == Some(parent.scope) => {}
            _ => return Err(AuthorityError::Unauthorized),
        }
        self.service.retire(child)?;
        self.issued.remove(&child);
        self.detached.remove(&child);
        Ok(())
    }

    /// Transport loss without retirement: the capability stays valid so the
    /// same client can reconnect and verify its outstanding work.
    pub(in crate::application::admission) fn disconnect(
        &mut self,
        scope: ScopeId,
        now: u64,
    ) -> Result<AbandonReport, AuthorityError> {
        let report = self.service.abandon(scope, now)?;
        if report.active > 0 {
            self.detached.insert(scope);
        }
        Ok(report)
    }

    /// Active attempts whose owners must terminate their local transport.
    pub(in crate::application::admission) fn cancellation_due(
        &mut self,
        now: u64,
    ) -> Result<Vec<RequestId>, AuthorityError> {
        let mut due = Vec::new();
        let ledger = self.service.ledger(now)?;
        for attempt in ledger.outstanding {
            let scope = ScopeId {
                epoch: ledger.epoch,
                serial: attempt.scope,
            };
            if !self.issued.contains_key(&scope) {
                continue;
            }
            if let RequestState::Active {
                cancellation_required: true,
                ..
            } = self.service.status(scope, attempt.sequence, now)?
            {
                due.push(RequestId {
                    scope,
                    sequence: attempt.sequence,
                });
            }
        }
        Ok(due)
    }

    pub(in crate::application::admission) fn next_wake(
        &mut self,
        now: u64,
    ) -> Result<Option<u64>, AuthorityError> {
        Ok(self.service.next_wake(now)?)
    }

    /// Whether the authority holds anything a stop would lose (#2615,
    /// ADR-0032): a root scope, a lost scope still owing work, or any
    /// permit, queued wait or lease, uncertain and restart-orphaned ones
    /// included. A state it cannot read counts as holding, so the idle stop
    /// fails safe.
    pub(in crate::application::admission) fn holds_anything(&mut self, now: u64) -> bool {
        let _ = now;
        true
    }

    pub fn inspect(&mut self, now: u64) -> AuthorityStatus {
        let groups = self
            .groups
            .iter()
            .filter_map(|group| {
                self.service
                    .snapshot(group, now)
                    .ok()
                    .map(|snapshot| (group.clone(), snapshot))
            })
            .collect();
        AuthorityStatus {
            epoch: self.service.ledger(now).map_or(0, |ledger| ledger.epoch),
            journal_healthy: self.journal_healthy,
            live_scopes: self.issued.len(),
            groups,
        }
    }

    /// The successor epoch's ledger an operator reset makes durable first.
    pub(in crate::application::admission) fn successor_ledger(
        &mut self,
        now: u64,
    ) -> Result<AdmissionLedger, AuthorityError> {
        let probe = self.service.ledger(now)?;
        let mut successor = probe.clone();
        successor.epoch = probe
            .epoch
            .checked_add(1)
            .ok_or(AuthorityError::Admission(AdmissionError::ScopeLimit))?;
        successor.outstanding.clear();
        Ok(successor)
    }

    /// Adopt the successor epoch once it is durable: every capability is
    /// revoked.
    pub(in crate::application::admission) fn adopt_successor(
        &mut self,
        durable_epoch: u64,
        now: u64,
    ) -> Result<u64, AuthorityError> {
        let epoch = self.service.reset(now)?;
        debug_assert_eq!(epoch, durable_epoch, "durable epoch matches policy");
        self.issued.clear();
        self.detached.clear();
        Ok(epoch)
    }
}
