//! #2220: the query hands the bounded selection its registry's session
//! and the inventory's diagnostics.

use std::sync::Arc;

use super::ListEnvironmentsQuery;
use crate::domain::environment_listing::ListingScope;
use crate::domain::environment_listing::tests::{CALLER, qa_record, qa_registry};
use crate::domain::environment_registry::{
    EnvironmentJournal, EnvironmentRegistry, EnvironmentStatus, JournalWrite,
};

#[test]
fn the_listing_is_selected_for_the_registrys_own_session() {
    let listing = ListEnvironmentsQuery::new(qa_registry()).listing(ListingScope::Actionable, 20);
    let shown: Vec<(&str, bool)> = listing
        .shown
        .iter()
        .map(|row| (row.record.environment_ref.as_str(), row.own))
        .collect();
    assert_eq!(
        shown,
        [("C13", false), ("C14", false), ("C15", true), ("C16", true)]
    );
    assert_eq!(
        (listing.total, listing.hidden, listing.omitted),
        (16, 12, 0)
    );
    assert!(listing.diagnostics.is_empty());
}

#[test]
fn an_earlier_run_of_this_session_is_listed_as_the_callers_own() {
    let registry = qa_registry();
    registry.restore(vec![qa_record(
        17,
        CALLER,
        EnvironmentStatus::CleanupFailed,
    )]);
    let listing = ListEnvironmentsQuery::new(registry).listing(ListingScope::Actionable, 20);
    let row = listing
        .shown
        .iter()
        .find(|row| row.record.environment_ref == "C17")
        .expect("the caller's own unsettled environment is listed");
    assert!(row.own);
    assert_eq!(listing.hidden, 12);
}

#[test]
fn the_listing_carries_the_registry_read_error_and_what_this_session_created() {
    let journal = EnvironmentJournal {
        allocate_ref: Arc::new(|_| Ok(1)),
        release_ref: Arc::new(|_| {}),
        recorded: Arc::new(|_, _| JournalWrite::Written),
        forgotten: Arc::new(|_| {}),
        reload: Arc::new(|| Err("environments.json: corrupt".into())),
    };
    let registry = EnvironmentRegistry::unreadable(journal, CALLER, "environments.json: corrupt");
    registry.commit(qa_record(1, CALLER, EnvironmentStatus::CleanupFailed));
    let query = ListEnvironmentsQuery::new(registry);
    let listing = query.listing(ListingScope::Actionable, 20);
    assert_eq!(listing.shown.len(), 1);
    assert!(listing.shown[0].own);
    assert_eq!(listing.diagnostics, query.diagnostics());
    assert_eq!(listing.diagnostics.len(), 1);
    assert!(listing.diagnostics[0].contains("environments.json: corrupt"));
    assert_eq!(query.execution_count(), 1);
}
