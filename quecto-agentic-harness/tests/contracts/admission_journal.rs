//! AdmissionJournal / AdmissionAuthority contract (#1679 P3 AC1/AC6): the
//! authority persists the ledger before any grant is visible and before a
//! completion is acknowledged, and fails closed when durability fails.
use quecto::application::inference_authority::{AdmissionAuthority, AuthorityError, Credential};
use quecto::application::ports::{AdmissionJournal, AdmissionSecretSource, JournalError};
use quecto::domain::inference_admission::{
    AdmissionConfig, AdmissionError, AdmissionLedger, Feedback, GroupId, GroupPolicy, RequestId,
    RequestState, TerminalOutcome, WorkloadClass,
};
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

#[derive(Default)]
struct JournalLog {
    writes: Vec<AdmissionLedger>,
    fail: bool,
}

#[derive(Clone, Default)]
struct FakeJournal(Rc<RefCell<JournalLog>>);

impl AdmissionJournal for FakeJournal {
    fn persist(&mut self, ledger: &AdmissionLedger) -> Result<(), JournalError> {
        let mut log = self.0.borrow_mut();
        if log.fail {
            return Err(JournalError::Unavailable("disk full".into()));
        }
        log.writes.push(ledger.clone());
        Ok(())
    }
}

struct Counter(u64);
impl AdmissionSecretSource for Counter {
    fn mint(&mut self) -> String {
        self.0 += 1;
        format!("secret-{}", self.0)
    }
}

fn config() -> AdmissionConfig {
    let g = GroupId::new("g").unwrap();
    AdmissionConfig {
        groups: BTreeMap::from([(
            g.clone(),
            GroupPolicy {
                capacity: 1,
                reserve: 0,
                min_interval_ms: 1,
                queue_capacity: 4,
                queue_timeout_ms: 1_000,
                attempt_timeout_ms: 5_000,
                fallback_base_ms: 100,
                max_cooldown_ms: 10_000,
            },
        )]),
        aliases: BTreeMap::from([("g".into(), g)]),
        max_scopes: 8,
        terminal_capacity: 16,
    }
}

fn group() -> GroupId {
    GroupId::new("g").unwrap()
}

type Authority = AdmissionAuthority<FakeJournal, Counter>;

fn authority() -> (Authority, FakeJournal) {
    let journal = FakeJournal::default();
    let authority = AdmissionAuthority::new(7, config(), journal.clone(), Counter(0)).unwrap();
    (authority, journal)
}

fn wrong(credential: &Credential) -> Credential {
    Credential {
        scope: credential.scope,
        token: "forged".into(),
    }
}

#[test]
fn roots_receive_distinct_secrets_and_forged_tokens_are_unauthorized() {
    let (mut a, _) = authority();
    let one = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    let two = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    assert_ne!(one.token, two.token, "distinct capabilities");
    assert_ne!(one.scope, two.scope);
    assert_eq!(
        a.acquire(&wrong(&one), 1, "g", 0).err(),
        Some(AuthorityError::Unauthorized)
    );
    assert_eq!(
        a.acquire(&one, 1, "g", 0).unwrap(),
        RequestState::Queued { deadline: 1_000 }
    );
}

#[test]
fn child_registration_requires_the_parent_capability_and_inherits_its_root() {
    let (mut a, _) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    assert_eq!(
        a.register_child(&wrong(&root), 0).err(),
        Some(AuthorityError::Unauthorized)
    );
    let child = a.register_child(&root, 0).unwrap();
    let grandchild = a.register_child(&child, 0).unwrap();
    assert_eq!(grandchild.scope.epoch, 7);
    assert!(grandchild.scope.serial > child.scope.serial);
    assert_eq!(
        a.lineage(grandchild.scope).unwrap(),
        root.scope,
        "descendants report their root"
    );
}

#[test]
fn a_grant_is_journaled_before_it_becomes_visible() {
    let (mut a, journal) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    a.acquire(&root, 1, "g", 0).unwrap();
    let grants = a.pump(0).unwrap();
    assert_eq!(
        grants,
        vec![RequestId {
            scope: root.scope,
            sequence: 1
        }]
    );
    let log = journal.0.borrow();
    let last = log.writes.last().expect("ledger persisted");
    assert_eq!(
        last.outstanding.len(),
        1,
        "outstanding attempt durable before grant"
    );
    assert_eq!(last.outstanding[0].sequence, 1);
    assert_eq!(last.epoch, 7);
}

#[test]
fn journal_failure_withdraws_the_grant_and_stops_new_grants_until_recovery() {
    let (mut a, journal) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    a.acquire(&root, 1, "g", 0).unwrap();
    journal.0.borrow_mut().fail = true;
    assert_eq!(a.pump(0).err(), Some(AuthorityError::JournalUnavailable));
    assert_eq!(
        a.status(&root, 1, 0).unwrap(),
        RequestState::Terminal(TerminalOutcome::Cancelled),
        "undelivered grant withdrawn"
    );
    assert!(!a.inspect(0).journal_healthy);
    a.acquire(&root, 2, "g", 1).unwrap();
    assert_eq!(a.pump(1).err(), Some(AuthorityError::JournalUnavailable));
    assert_eq!(
        a.status(&root, 2, 1).unwrap(),
        RequestState::Terminal(TerminalOutcome::Cancelled)
    );
    journal.0.borrow_mut().fail = false;
    a.acquire(&root, 3, "g", 2).unwrap();
    assert_eq!(
        a.pump(2).unwrap().len(),
        1,
        "grants resume after a durable write"
    );
    assert!(a.inspect(2).journal_healthy);
}

#[test]
fn completion_persists_the_release_before_acknowledging() {
    let (mut a, journal) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    a.acquire(&root, 1, "g", 0).unwrap();
    a.pump(0).unwrap();
    a.complete(&root, 1, Feedback::Success, 1).unwrap();
    let log = journal.0.borrow();
    assert!(
        log.writes.last().unwrap().outstanding.is_empty(),
        "release durable at acknowledgement"
    );
    drop(log);
    journal.0.borrow_mut().fail = true;
    a.acquire(&root, 2, "g", 2).unwrap();
    journal.0.borrow_mut().fail = false;
    a.pump(2).unwrap();
    journal.0.borrow_mut().fail = true;
    assert_eq!(
        a.complete(&root, 2, Feedback::Success, 3).err(),
        Some(AuthorityError::JournalUnavailable),
        "completion is not acknowledged without a durable release"
    );
}

#[test]
fn disconnect_cancels_queued_work_and_quarantines_active_work() {
    let (mut a, _) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    let other = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    a.acquire(&root, 1, "g", 0).unwrap();
    a.pump(0).unwrap();
    a.acquire(&root, 2, "g", 0).unwrap();
    a.acquire(&other, 1, "g", 0).unwrap();
    let report = a.disconnect(root.scope, 1).unwrap();
    assert_eq!((report.cancelled, report.uncertain), (1, 1));
    assert_eq!(
        a.pump(1).err(),
        Some(AuthorityError::Admission(AdmissionError::Quarantined))
    );
    let status = a.inspect(1);
    assert_eq!(status.groups[&group()].uncertain, 1);
    // The same capability reconnecting and completing clears the quarantine.
    a.complete(&root, 1, Feedback::Success, 2).unwrap();
    assert_eq!(a.pump(2).unwrap().len(), 1);
}

#[test]
fn repeated_acquire_reconciles_the_existing_request() {
    let (mut a, _) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    a.acquire(&root, 1, "g", 0).unwrap();
    a.pump(0).unwrap();
    assert!(matches!(
        a.acquire(&root, 1, "g", 1).unwrap(),
        RequestState::Active { .. }
    ));
    assert_eq!(a.pump(1).unwrap(), vec![], "no second grant");
}

#[test]
fn reset_revokes_every_capability_and_starts_a_new_epoch() {
    let (mut a, journal) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    a.acquire(&root, 1, "g", 0).unwrap();
    a.pump(0).unwrap();
    a.disconnect(root.scope, 1).unwrap();
    assert_eq!(a.reset(2).unwrap(), 8);
    assert_eq!(
        a.status(&root, 1, 2).err(),
        Some(AuthorityError::Unauthorized),
        "old capability revoked"
    );
    assert_eq!(
        journal.0.borrow().writes.last().unwrap().epoch,
        8,
        "reset is durable"
    );
    let fresh = a.register_root(WorkloadClass::Interactive, 3).unwrap();
    assert_eq!(fresh.scope.epoch, 8);
    a.acquire(&fresh, 1, "g", 3).unwrap();
    assert_eq!(a.pump(3).unwrap().len(), 1);
}

#[test]
fn reset_is_refused_when_the_journal_cannot_record_it() {
    let (mut a, journal) = authority();
    journal.0.borrow_mut().fail = true;
    assert_eq!(a.reset(1).err(), Some(AuthorityError::JournalUnavailable));
    assert_eq!(a.inspect(1).epoch, 7, "epoch unchanged");
}

#[test]
fn cancellation_due_lists_only_active_attempts_that_must_stop() {
    let (mut a, _) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    a.acquire(&root, 1, "g", 0).unwrap();
    a.pump(0).unwrap();
    assert_eq!(a.cancellation_due(1).unwrap(), vec![]);
    a.cancel(&root, 1, 2).unwrap();
    assert_eq!(
        a.cancellation_due(2).unwrap(),
        vec![RequestId {
            scope: root.scope,
            sequence: 1
        }]
    );
    a.complete(&root, 1, Feedback::Failure, 3).unwrap();
    assert_eq!(a.cancellation_due(3).unwrap(), vec![]);
}

#[test]
fn retire_releases_a_scope_and_its_capability() {
    let (mut a, _) = authority();
    let root = a.register_root(WorkloadClass::Interactive, 0).unwrap();
    a.retire(&root, 0).unwrap();
    assert_eq!(
        a.acquire(&root, 1, "g", 1).err(),
        Some(AuthorityError::Unauthorized)
    );
}
