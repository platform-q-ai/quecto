use super::*;
use crate::domain::environments::entities::environment_journal::{
    EnvironmentJournal, JournalWrite,
};
use std::sync::{Arc, Mutex};

/// A registry whose journal allocates C1, C2, … and records releases.
fn journalled() -> (EnvironmentRegistry, Arc<Mutex<Vec<u64>>>) {
    let released = Arc::new(Mutex::new(Vec::new()));
    let next = Arc::new(Mutex::new(0u64));
    let journal = EnvironmentJournal {
        allocate_ref: Arc::new(move |_floor| {
            let mut next = next.lock().unwrap();
            *next += 1;
            Ok(*next)
        }),
        release_ref: Arc::new({
            let released = released.clone();
            move |number| released.lock().unwrap().push(number)
        }),
        recorded: Arc::new(|_, _| JournalWrite::Written),
        forgotten: Arc::new(|_| {}),
        reload: Arc::new(|| Ok(Vec::new())),
    };
    (
        EnvironmentRegistry::with_journal(journal, "cli:test"),
        released,
    )
}

#[test]
fn an_uncommitted_ref_goes_back_when_dropped() {
    let (registry, released) = journalled();
    let minted = MintedRef::new(registry.mint_ref().unwrap(), registry.clone());
    assert_eq!(minted.as_str(), "C1");
    drop(minted);
    assert_eq!(*released.lock().unwrap(), vec![1]);
}

#[test]
fn a_committed_ref_stays_with_its_record() {
    let (registry, released) = journalled();
    let minted = MintedRef::new(registry.mint_ref().unwrap(), registry.clone());
    minted.commit(super::super::tests::test_record("C1", "env-1"));
    assert!(released.lock().unwrap().is_empty());
    assert!(registry.get("C1").is_some());
}
