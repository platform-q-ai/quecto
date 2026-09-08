//! Authenticated, durability-gated admission authority over the P1 service.
//!
//! Every client operation carries a capability issued here. Grants become
//! visible only after the ledger is durable; completions are acknowledged only
//! after the release is durable. Failure of either path fails closed.
use std::collections::BTreeMap;

use super::inference_admission::AdmissionService;
use super::ports::{
    AdmissionClient, AdmissionDispatcher, AdmissionJournal, AdmissionRecovery, AdmissionRegistry,
    AdmissionSecretSource, JournalError,
};
use crate::domain::inference_admission::*;

/// Scope identity plus the opaque secret proving the caller was issued it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credential {
    pub scope: ScopeId,
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthorityError {
    Unauthorized,
    Admission(AdmissionError),
    JournalUnavailable,
}

impl From<AdmissionError> for AuthorityError {
    fn from(error: AdmissionError) -> Self {
        Self::Admission(error)
    }
}

impl From<JournalError> for AuthorityError {
    fn from(_: JournalError) -> Self {
        Self::JournalUnavailable
    }
}

/// Operator-facing state; contains no capability material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityStatus {
    pub epoch: u64,
    pub journal_healthy: bool,
    pub groups: BTreeMap<GroupId, GroupSnapshot>,
}

#[derive(Debug)]
struct Issued {
    token: String,
    root: ScopeId,
}

#[derive(Debug)]
pub struct AdmissionAuthority<J, S> {
    service: AdmissionService,
    journal: J,
    secrets: S,
    issued: BTreeMap<ScopeId, Issued>,
    groups: Vec<GroupId>,
    journal_healthy: bool,
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut diff = a.len() ^ b.len();
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= usize::from(x ^ y);
    }
    diff == 0
}

impl<J: AdmissionJournal, S: AdmissionSecretSource> AdmissionAuthority<J, S> {
    pub fn new(
        epoch: u64,
        config: AdmissionConfig,
        journal: J,
        secrets: S,
    ) -> Result<Self, AuthorityError> {
        let groups = config.groups.keys().cloned().collect();
        Ok(Self::compose(
            AdmissionService::new(epoch, config)?,
            journal,
            secrets,
            groups,
        ))
    }

    /// Rehydrate at the ledger's epoch with outstanding work orphaned.
    pub fn restore(
        config: AdmissionConfig,
        ledger: &AdmissionLedger,
        journal: J,
        secrets: S,
        now: u64,
    ) -> Result<Self, AuthorityError> {
        let groups = config.groups.keys().cloned().collect();
        Ok(Self::compose(
            AdmissionService::restore(config, ledger, now)?,
            journal,
            secrets,
            groups,
        ))
    }

    fn compose(service: AdmissionService, journal: J, secrets: S, groups: Vec<GroupId>) -> Self {
        Self {
            service,
            journal,
            secrets,
            issued: BTreeMap::new(),
            groups,
            journal_healthy: true,
        }
    }

    fn authenticate(&self, credential: &Credential) -> Result<(), AuthorityError> {
        match self.issued.get(&credential.scope) {
            Some(issued) if constant_time_eq(&issued.token, &credential.token) => Ok(()),
            _ => Err(AuthorityError::Unauthorized),
        }
    }

    fn issue(&mut self, scope: ScopeId, root: ScopeId) -> Credential {
        let token = self.secrets.mint();
        debug_assert!(!token.is_empty(), "secret source must mint material");
        self.issued.insert(
            scope,
            Issued {
                token: token.clone(),
                root,
            },
        );
        Credential { scope, token }
    }

    fn persist(&mut self, now: u64) -> Result<(), AuthorityError> {
        let ledger = self.service.ledger(now)?;
        match self.journal.persist(&ledger) {
            Ok(()) => {
                self.journal_healthy = true;
                Ok(())
            }
            Err(_) => {
                self.journal_healthy = false;
                Err(AuthorityError::JournalUnavailable)
            }
        }
    }

    pub fn register_root(
        &mut self,
        class: WorkloadClass,
        now: u64,
    ) -> Result<Credential, AuthorityError> {
        let _ = now;
        let scope = self.service.register_root(class)?;
        Ok(self.issue(scope, scope))
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
        Ok(self.issue(scope, root))
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

    /// Dispatch every grant the policy allows, making each durable first. On a
    /// journal failure the undelivered grants are withdrawn and no grant is
    /// returned until a later durable write succeeds.
    pub fn pump(&mut self, now: u64) -> Result<Vec<RequestId>, AuthorityError> {
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
        if grants.is_empty() {
            return match first_error {
                Some(error) => Err(error.into()),
                None => Ok(grants),
            };
        }
        if self.persist(now).is_err() {
            for id in &grants {
                self.service.withdraw(id.scope, id.sequence, now)?;
            }
            return Err(AuthorityError::JournalUnavailable);
        }
        Ok(grants)
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

    /// The release is durable before it is acknowledged; a failed write leaves
    /// the in-memory release but reports the failure so the client retries.
    pub fn complete(
        &mut self,
        credential: &Credential,
        sequence: u64,
        feedback: Feedback,
        now: u64,
    ) -> Result<RequestState, AuthorityError> {
        self.authenticate(credential)?;
        let state = self
            .service
            .complete(credential.scope, sequence, feedback, now)?;
        self.persist(now)?;
        Ok(state)
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
        Ok(())
    }

    /// Transport loss without retirement: the capability stays valid so the
    /// same client can reconnect and verify its outstanding work.
    pub fn disconnect(
        &mut self,
        scope: ScopeId,
        now: u64,
    ) -> Result<AbandonReport, AuthorityError> {
        Ok(self.service.abandon(scope, now)?)
    }

    /// Active attempts whose owners must terminate their local transport.
    pub fn cancellation_due(&mut self, now: u64) -> Result<Vec<RequestId>, AuthorityError> {
        let mut due = Vec::new();
        for attempt in self.service.ledger(now)?.outstanding {
            let scope = ScopeId {
                epoch: self.inspect_epoch(),
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

    pub fn next_wake(&mut self, now: u64) -> Result<Option<u64>, AuthorityError> {
        Ok(self.service.next_wake(now)?)
    }

    fn inspect_epoch(&mut self) -> u64 {
        self.service
            .ledger(0)
            .map(|ledger| ledger.epoch)
            .unwrap_or_else(|_| self.epoch_hint())
    }

    fn epoch_hint(&self) -> u64 {
        self.issued.keys().next().map_or(0, |scope| scope.epoch)
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
            groups,
        }
    }

    /// Operator reset: durable successor epoch, every capability revoked.
    pub fn reset(&mut self, now: u64) -> Result<u64, AuthorityError> {
        let probe = self.service.ledger(now)?;
        let mut successor = probe.clone();
        successor.epoch = probe
            .epoch
            .checked_add(1)
            .ok_or(AuthorityError::Admission(AdmissionError::ScopeLimit))?;
        successor.outstanding.clear();
        if self.journal.persist(&successor).is_err() {
            self.journal_healthy = false;
            return Err(AuthorityError::JournalUnavailable);
        }
        self.journal_healthy = true;
        let epoch = self.service.reset(now)?;
        debug_assert_eq!(epoch, successor.epoch, "durable epoch matches policy");
        self.issued.clear();
        Ok(epoch)
    }
}
