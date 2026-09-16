//! Contract for the `DelegatedChildrenRoster` port (#1938, D7 #1976): a
//! live delegated row is one this harness addresses as a delegated agent
//! (it carries a launch or reported generation), still live and not
//! exited; a record-only row, a detached row and an exited tombstone are
//! not counted. `clear_roster` drops every row and reports how many.
use std::sync::Arc;

use quecto::application::sessions::ports::DelegatedChildrenRoster;
use quecto::domain::session::SubagentLiveness;
use quecto::domain::subagent_teardown::LaunchGeneration;
use quecto::infrastructure::tools::delegated_roster::RegistryDelegatedRoster;
use quecto::infrastructure::tools::subagent_registry::{
    SubagentEntry, SubagentRegistry, SubagentStatus, new_registry,
};

fn under_test(registry: SubagentRegistry) -> Arc<dyn DelegatedChildrenRoster> {
    Arc::new(RegistryDelegatedRoster::new(registry))
}

fn delegated(generation: u64) -> SubagentEntry {
    let mut entry = SubagentEntry::new("/tmp/child.sock".into(), 0);
    entry.launch_generation = Some(LaunchGeneration::new(generation));
    entry
}

#[test]
fn an_empty_registry_has_no_live_rows_and_nothing_to_clear() {
    let roster = under_test(new_registry());
    assert_eq!(roster.live_delegated_rows(), 0);
    assert_eq!(roster.clear_roster(), 0);
}

#[test]
fn only_live_non_exited_delegated_rows_count() {
    let registry = new_registry();
    {
        let mut entries = registry.lock().unwrap();
        entries.insert("live".into(), delegated(1));
        let mut exited = delegated(2);
        exited.status = SubagentStatus::Exited;
        entries.insert("exited".into(), exited);
        let mut detached = delegated(3);
        detached.persisted_liveness = SubagentLiveness::Detached;
        entries.insert("detached".into(), detached);
        entries.insert(
            "record".into(),
            SubagentEntry::new("/tmp/record.sock".into(), 0),
        );
    }
    let roster = under_test(registry.clone());
    assert_eq!(roster.live_delegated_rows(), 1);
    assert_eq!(roster.clear_roster(), 4, "every row leaves the roster");
    assert!(registry.lock().unwrap().is_empty());
    assert_eq!(roster.live_delegated_rows(), 0);
}
