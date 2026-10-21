//! What a settling turn reports about holding (#2615, ADR-0032): only an
//! authority that holds nothing a stop would lose may stop itself when idle.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use super::*;
use crate::application::admission::dto::broker::Credential;
use crate::application::admission::ports::inference_authority_ports::{
    AdmissionJournal, JournalError, SharedJournal,
};
use crate::application::admission::use_cases::DisconnectScope;
use crate::domain::admission::value_objects::inference_admission::{
    AdmissionConfig, AdmissionLedger, Feedback, GroupId, GroupPolicy, WorkloadClass,
};

struct MemoryJournal;

impl AdmissionJournal for MemoryJournal {
    fn persist(&mut self, _ledger: &AdmissionLedger) -> Result<(), JournalError> {
        Ok(())
    }

    fn load(&mut self) -> Result<Option<AdmissionLedger>, JournalError> {
        Ok(None)
    }
}

struct Counter(u64);

impl AdmissionSecretSource for Counter {
    fn mint(&mut self) -> String {
        self.0 += 1;
        format!("fake-secret-{}", self.0)
    }
}

fn config() -> AdmissionConfig {
    paced(1)
}

/// The policy with `min_interval_ms` between request starts.
fn paced(min_interval_ms: u64) -> AdmissionConfig {
    let group = GroupId::new("g").unwrap();
    AdmissionConfig {
        groups: BTreeMap::from([(
            group.clone(),
            GroupPolicy {
                capacity: 1,
                reserve: 0,
                min_interval_ms,
                queue_capacity: 8,
                queue_timeout_ms: 60_000,
                attempt_timeout_ms: 60_000,
                fallback_base_ms: 50,
                max_cooldown_ms: 500,
            },
        )]),
        aliases: BTreeMap::from([("acct".into(), group)]),
        max_scopes: 8,
        terminal_capacity: 16,
    }
}

fn settle() -> SettleAuthority {
    let journal: SharedJournal = Arc::new(Mutex::new(MemoryJournal));
    SettleAuthority::new(DispatchAdmissions::new(journal))
}

fn authority() -> AdmissionAuthority<Counter> {
    AdmissionAuthority::new(1, config(), Counter(0)).unwrap()
}

#[test]
fn a_fresh_authority_holds_nothing() {
    let mut authority = authority();
    assert!(!settle().execute(&mut authority, &[], 0).holding);
}

#[test]
fn a_registered_root_is_held_until_it_is_released() {
    let settle = settle();
    let mut authority = authority();
    let root = authority
        .register_root(WorkloadClass::Interactive, 0)
        .unwrap();
    assert!(settle.execute(&mut authority, &[], 1).holding);
    authority.retire(&root, 2).unwrap();
    assert!(!settle.execute(&mut authority, &[], 3).holding);
}

#[test]
fn a_child_scope_alone_is_not_a_root() {
    let settle = settle();
    let mut authority = authority();
    let root = authority
        .register_root(WorkloadClass::Interactive, 0)
        .unwrap();
    let _child = authority.register_child(&root, 0).unwrap();
    let released = DisconnectScope::new().execute(&mut authority, root.scope, 1);
    assert!(released.is_ok(), "{released:?}");
    assert!(!settle.execute(&mut authority, &[], 2).holding);
}

#[test]
fn a_permit_a_queued_wait_and_an_uncertain_lease_are_held() {
    let settle = settle();
    let mut authority = authority();
    let first = authority
        .register_root(WorkloadClass::Interactive, 0)
        .unwrap();
    let second = authority
        .register_root(WorkloadClass::Interactive, 0)
        .unwrap();
    authority.acquire(&first, 1, "acct", 0).unwrap();
    authority.acquire(&second, 1, "acct", 0).unwrap();
    assert!(
        settle.execute(&mut authority, &[], 1).holding,
        "a permit and a wait"
    );
    // Both clients are lost: the wait is cancelled, the permit becomes an
    // uncertain lease the broker must keep holding.
    DisconnectScope::new()
        .execute(&mut authority, second.scope, 2)
        .unwrap();
    DisconnectScope::new()
        .execute(&mut authority, first.scope, 2)
        .unwrap();
    let snapshot = authority.inspect(3);
    assert_eq!(snapshot.groups[&GroupId::new("g").unwrap()].uncertain, 1);
    assert!(
        settle.execute(&mut authority, &[], 3).holding,
        "an uncertain lease"
    );
}

#[test]
fn a_restart_orphan_is_held() {
    let settle = settle();
    let mut before = authority();
    let root = before.register_root(WorkloadClass::Interactive, 0).unwrap();
    before.acquire(&root, 1, "acct", 0).unwrap();
    settle.execute(&mut before, &[], 1);
    let ledger = before.ledger(1).unwrap();
    assert_eq!(ledger.outstanding.len(), 1, "the permit is outstanding");
    let mut restored = AdmissionAuthority::restore(config(), &ledger, Counter(0), 0).unwrap();
    assert!(settle.execute(&mut restored, &[], 1).holding);
}

/// A child scope whose root has gone: each kind of occupancy alone keeps the
/// authority holding, with no root registered (#2615 review).
fn orphaned_child(authority: &mut AdmissionAuthority<Counter>) -> Credential {
    let root = authority
        .register_root(WorkloadClass::Interactive, 0)
        .unwrap();
    let child = authority.register_child(&root, 0).unwrap();
    DisconnectScope::new()
        .execute(authority, root.scope, 0)
        .unwrap();
    child
}

#[test]
fn a_permit_alone_is_held() {
    let settle = settle();
    let mut authority = authority();
    let child = orphaned_child(&mut authority);
    assert!(!settle.execute(&mut authority, &[], 1).holding, "no root");
    authority.acquire(&child, 1, "acct", 1).unwrap();
    settle.execute(&mut authority, &[], 2);
    let group = authority.inspect(2).groups[&GroupId::new("g").unwrap()];
    assert_eq!((group.active, group.queued, group.uncertain), (1, 0, 0));
    assert!(settle.execute(&mut authority, &[], 3).holding);
}

#[test]
fn a_queued_wait_alone_is_held() {
    let settle = settle();
    let mut authority = AdmissionAuthority::new(1, paced(60_000), Counter(0)).unwrap();
    let child = orphaned_child(&mut authority);
    authority.acquire(&child, 1, "acct", 1).unwrap();
    settle.execute(&mut authority, &[], 1);
    crate::application::admission::use_cases::CompleteAdmission::new(Arc::new(Mutex::new(
        MemoryJournal,
    )))
    .execute(&mut authority, &child, 1, Feedback::Success, 2)
    .unwrap();
    assert!(!settle.execute(&mut authority, &[], 3).holding, "all done");
    // Paced: the next start waits a minute, with nothing outstanding.
    authority.acquire(&child, 2, "acct", 4).unwrap();
    settle.execute(&mut authority, &[], 5);
    let group = authority.inspect(5).groups[&GroupId::new("g").unwrap()];
    assert_eq!((group.active, group.queued, group.uncertain), (0, 1, 0));
    assert!(settle.execute(&mut authority, &[], 6).holding);
}
