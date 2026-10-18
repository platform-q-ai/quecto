//! #2220: the bounded listing's selection — what a caller can act on, in
//! ref-number order, capped with counts for what is left out — and the
//! registry it was found on, shared with the query's and the tool
//! adapter's tests.

use std::path::PathBuf;
use std::sync::Arc;

use super::{EnvironmentListing, ListingScope, select_listing};
use crate::domain::environments::entities::environment_registry::{
    EnvironmentJournal, EnvironmentOrigin, EnvironmentRecord, EnvironmentRegistry,
    EnvironmentStatus, JournalWrite,
};

/// The session the fixture's registry lists for.
pub(crate) const CALLER: &str = "cli:qa5-h-3";

/// A journal that accepts every write and allocates nothing the tests use:
/// just enough for a registry that knows its session.
pub(crate) fn quiet_journal() -> EnvironmentJournal {
    EnvironmentJournal {
        allocate_ref: Arc::new(|floor| Ok(floor + 1)),
        release_ref: Arc::new(|_| {}),
        recorded: Arc::new(|_, _| JournalWrite::Written),
        forgotten: Arc::new(|_| {}),
        reload: Arc::new(|| Ok(Vec::new())),
    }
}

/// One record as the shipped create script leaves it: `n` names the ref
/// and the state directory, `session` the creator.
pub(crate) fn qa_record(n: u64, session: &str, status: EnvironmentStatus) -> EnvironmentRecord {
    let workspace = format!("/home/qa/.quecto/containers/standard/state/env-q{n:09}/workspace");
    let repository = "https://github.com/platform-q-ai/quecto.git";
    EnvironmentRecord {
        environment_ref: format!("C{n}"),
        environment_id: format!("env-q{n:09}"),
        environment_uuid: format!("5f0c8a52-1d3e-4c6b-9a7e-{n:012}"),
        name: None,
        workspace_path: PathBuf::from(&workspace),
        repository: repository.to_string(),
        script_name: "standard".to_string(),
        retained_exec_argv: vec!["exec.sh".into()],
        retained_kill_argv: vec!["kill.sh".into()],
        retained_cleanup_argv: vec!["kill.sh".into(), "--cleanup".into()],
        retained_inspect_argv: vec!["inspect.sh".into()],
        members: Vec::new(),
        status,
        metadata: serde_json::json!({
            "runtime": "podman",
            "config": "standard",
            "source": "repo",
            "checkout": format!("{workspace}/repo"),
            "repository": repository,
        }),
        last_error: None,
        origin: EnvironmentOrigin::Created,
        created_by: session.to_string(),
        created_at: Some(1_790_000_000 + n * 60),
    }
}

/// The #2220 registry: C1–C12 stopped and C13–C14 empty, restored from
/// earlier sessions; C15 and C16 running with one member each, created by
/// [`CALLER`].
pub(crate) fn qa_registry() -> EnvironmentRegistry {
    let registry = EnvironmentRegistry::with_journal(quiet_journal(), CALLER);
    let mut restored = Vec::new();
    for n in 1..=12 {
        let mut record = qa_record(
            n,
            &format!("cli:qa5-h-{}", n % 2 + 1),
            EnvironmentStatus::Stopped,
        );
        record.last_error =
            Some("container not found at restore: the runtime reports it gone".to_string());
        restored.push(record);
    }
    for n in 13..=14 {
        restored.push(qa_record(n, "cli:qa5-h-2", EnvironmentStatus::Running));
    }
    registry.restore(restored);
    for n in 15..=16 {
        let mut record = qa_record(n, CALLER, EnvironmentStatus::Running);
        record.members = vec![format!("7d1e2c3b-4a5f-4e6d-8c7b-{n:012}")];
        registry.commit(record);
    }
    registry
}

/// The selection over a registry's snapshot for its own session.
fn select(registry: &EnvironmentRegistry, scope: ListingScope, limit: usize) -> EnvironmentListing {
    select_listing(registry.entries(), registry.session(), scope, limit)
}

fn refs(listing: &EnvironmentListing) -> Vec<&str> {
    listing
        .shown
        .iter()
        .map(|row| row.record.environment_ref.as_str())
        .collect()
}

fn own(listing: &EnvironmentListing) -> Vec<bool> {
    listing.shown.iter().map(|row| row.own).collect()
}

#[test]
fn by_default_only_joinable_and_own_environments_are_listed_and_the_rest_is_counted() {
    let listing = select(&qa_registry(), ListingScope::Actionable, 20);
    assert_eq!(refs(&listing), ["C13", "C14", "C15", "C16"]);
    assert_eq!(own(&listing), [false, false, true, true]);
    assert_eq!(listing.total, 16);
    assert_eq!(listing.hidden, 12);
    assert_eq!(listing.omitted, 0);
}

#[test]
fn all_lists_every_record_in_ref_number_order() {
    let listing = select(&qa_registry(), ListingScope::All, 20);
    let expected: Vec<String> = (1..=16).map(|n| format!("C{n}")).collect();
    assert_eq!(refs(&listing), expected);
    assert_eq!((listing.total, listing.hidden, listing.omitted), (16, 0, 0));
}

#[test]
fn the_cap_keeps_tier_by_tier_newest_first_and_counts_the_rest() {
    let listing = select(&qa_registry(), ListingScope::All, 5);
    // C15–C16 (created here) and C13–C14 (joinable) survive the cap; the
    // newest other ref fills the last row; all shown in ref-number order.
    assert_eq!(refs(&listing), ["C12", "C13", "C14", "C15", "C16"]);
    assert_eq!(
        (listing.total, listing.hidden, listing.omitted),
        (16, 0, 11)
    );

    // The caller's own running C15 and C16 are never the ones cut.
    let listing = select(&qa_registry(), ListingScope::Actionable, 3);
    assert_eq!(refs(&listing), ["C14", "C15", "C16"]);
    assert_eq!(
        (listing.total, listing.hidden, listing.omitted),
        (16, 12, 1)
    );
}

#[test]
fn own_environments_are_listed_unless_stopped_and_no_foreign_stale_one_is() {
    let registry = EnvironmentRegistry::with_journal(quiet_journal(), CALLER);
    let mut foreign = Vec::new();
    for (n, status) in [
        (1, EnvironmentStatus::Killing),
        (2, EnvironmentStatus::CleanupFailed),
        (3, EnvironmentStatus::Stopped),
        (4, EnvironmentStatus::Retained),
    ] {
        foreign.push(qa_record(n, "cli:elsewhere", status));
    }
    // Earlier runs of this very session: restored, the caller's own —
    // listed while unsettled, hidden once stopped.
    foreign.push(qa_record(5, CALLER, EnvironmentStatus::Stopped));
    foreign.push(qa_record(6, CALLER, EnvironmentStatus::CleanupFailed));
    foreign.push(qa_record(7, CALLER, EnvironmentStatus::Killing));
    registry.restore(foreign);
    for (n, status) in [
        (10, EnvironmentStatus::Stopped),
        (11, EnvironmentStatus::CleanupFailed),
        (12, EnvironmentStatus::Killing),
    ] {
        registry.commit(qa_record(n, CALLER, status));
    }
    let listing = select(&registry, ListingScope::Actionable, 20);
    // Stopped ones — C3, C5, and this process's own C10 — are hidden.
    assert_eq!(refs(&listing), ["C4", "C6", "C7", "C11", "C12"]);
    assert_eq!(own(&listing), [false, true, true, true, true]);
    assert_eq!((listing.total, listing.hidden, listing.omitted), (10, 5, 0));
    let listing = select(&registry, ListingScope::All, 20);
    let c5 = listing
        .shown
        .iter()
        .find(|row| row.record.environment_ref == "C5")
        .expect("all lists the caller's restored stopped C5");
    assert!(c5.own);
    assert!(
        refs(&listing).contains(&"C10"),
        "all lists this process's stopped C10"
    );
}

#[test]
fn twenty_foreign_running_environments_never_crowd_out_the_callers_own() {
    let registry = EnvironmentRegistry::with_journal(quiet_journal(), CALLER);
    registry.restore(
        (1..=20)
            .map(|n| qa_record(n, "cli:elsewhere", EnvironmentStatus::Running))
            .collect(),
    );
    registry.commit(qa_record(30, CALLER, EnvironmentStatus::Running));
    let listing = select(&registry, ListingScope::Actionable, 20);
    let shown = refs(&listing);
    assert!(shown.contains(&"C30"), "{shown:?}");
    assert_eq!(shown.len(), 20);
    // Newest first within the joinable tier: C1 is the one cut.
    assert!(!shown.contains(&"C1"), "{shown:?}");
    assert_eq!((listing.total, listing.hidden, listing.omitted), (21, 0, 1));
}

#[test]
fn restored_stopped_own_environments_are_hidden_and_never_crowd_out_a_new_one() {
    let registry = EnvironmentRegistry::with_journal(quiet_journal(), CALLER);
    registry.restore(
        (1..=20)
            .map(|n| qa_record(n, CALLER, EnvironmentStatus::Stopped))
            .collect(),
    );
    registry.commit(qa_record(21, CALLER, EnvironmentStatus::Running));
    let listing = select(&registry, ListingScope::Actionable, 20);
    assert_eq!(refs(&listing), ["C21"]);
    assert_eq!(
        (listing.total, listing.hidden, listing.omitted),
        (21, 20, 0)
    );
    let listing = select(&registry, ListingScope::All, 20);
    let shown = refs(&listing);
    assert!(shown.contains(&"C21"), "{shown:?}");
    assert_eq!(listing.omitted, 1);
}

#[test]
fn every_tier_outranks_the_next_when_the_cap_bites() {
    let registry = EnvironmentRegistry::with_journal(quiet_journal(), CALLER);
    registry.restore(vec![
        qa_record(10, "cli:elsewhere", EnvironmentStatus::Stopped),
        qa_record(30, "cli:elsewhere", EnvironmentStatus::Retained),
        qa_record(40, CALLER, EnvironmentStatus::CleanupFailed),
    ]);
    registry.commit(qa_record(20, CALLER, EnvironmentStatus::Stopped));
    registry.commit(qa_record(50, CALLER, EnvironmentStatus::Running));
    // Created live, own restored live, foreign joinable, created stopped,
    // the rest.
    for (limit, expected) in [
        (1, vec!["C50"]),
        (2, vec!["C40", "C50"]),
        (3, vec!["C30", "C40", "C50"]),
        (4, vec!["C20", "C30", "C40", "C50"]),
        (5, vec!["C10", "C20", "C30", "C40", "C50"]),
    ] {
        let listing = select(&registry, ListingScope::All, limit);
        assert_eq!(refs(&listing), expected, "limit {limit}");
    }
    let listing = select(&registry, ListingScope::All, 5);
    let ranks: Vec<usize> = listing.shown.iter().map(|row| row.keep_rank).collect();
    assert_eq!(ranks, [4, 3, 2, 1, 0], "C10 is the first a cap drops");
    let listing = select(&registry, ListingScope::Actionable, 5);
    assert_eq!(refs(&listing), ["C30", "C40", "C50"]);
}

#[test]
fn a_long_session_s_stopped_children_never_crowd_out_its_live_swarm() {
    let registry = EnvironmentRegistry::with_journal(quiet_journal(), CALLER);
    registry.commit(qa_record(3, CALLER, EnvironmentStatus::Running));
    for n in 4..=23 {
        registry.commit(qa_record(n, CALLER, EnvironmentStatus::Stopped));
    }
    let listing = select(&registry, ListingScope::Actionable, 20);
    assert_eq!(refs(&listing), ["C3"]);
    assert_eq!(
        (listing.total, listing.hidden, listing.omitted),
        (21, 20, 0)
    );
    let listing = select(&registry, ListingScope::All, 20);
    let shown = refs(&listing);
    assert!(shown.contains(&"C3"), "{shown:?}");
    assert!(
        !shown.contains(&"C4"),
        "the oldest stopped is cut: {shown:?}"
    );
    assert_eq!(listing.omitted, 1);
}

#[test]
fn the_caller_s_restored_live_environments_outrank_foreign_joinable_ones() {
    let registry = EnvironmentRegistry::with_journal(quiet_journal(), CALLER);
    let mut restored: Vec<_> = (1..=20)
        .map(|n| qa_record(n, "cli:elsewhere", EnvironmentStatus::Running))
        .collect();
    restored.push(qa_record(30, CALLER, EnvironmentStatus::CleanupFailed));
    restored.push(qa_record(31, CALLER, EnvironmentStatus::Retained));
    registry.restore(restored);
    let listing = select(&registry, ListingScope::Actionable, 20);
    let shown = refs(&listing);
    assert!(
        shown.contains(&"C30") && shown.contains(&"C31"),
        "{shown:?}"
    );
    assert!(
        !shown.contains(&"C1") && !shown.contains(&"C2"),
        "{shown:?}"
    );
    assert_eq!((listing.total, listing.hidden, listing.omitted), (22, 0, 2));
}

#[test]
fn the_caller_s_restored_live_environments_survive_even_with_the_oldest_refs() {
    // Newest-first alone would cut C1 and C2: only their tier keeps them.
    let registry = EnvironmentRegistry::with_journal(quiet_journal(), CALLER);
    let mut restored: Vec<_> = (10..=29)
        .map(|n| qa_record(n, "cli:elsewhere", EnvironmentStatus::Running))
        .collect();
    restored.push(qa_record(1, CALLER, EnvironmentStatus::CleanupFailed));
    restored.push(qa_record(2, CALLER, EnvironmentStatus::Retained));
    registry.restore(restored);
    let listing = select(&registry, ListingScope::Actionable, 20);
    let shown = refs(&listing);
    assert_eq!(shown[..2], ["C1", "C2"], "{shown:?}");
    assert!(
        !shown.contains(&"C10") && !shown.contains(&"C11"),
        "{shown:?}"
    );
}

#[test]
fn a_session_less_registry_owns_only_what_it_created() {
    let registry = EnvironmentRegistry::new();
    registry.restore(vec![qa_record(1, "", EnvironmentStatus::Stopped)]);
    registry.commit(qa_record(2, "", EnvironmentStatus::Stopped));
    let listing = select(&registry, ListingScope::All, 20);
    assert_eq!(refs(&listing), ["C1", "C2"]);
    assert_eq!(own(&listing), [false, true]);
}

#[test]
fn refs_that_are_not_numbered_sort_after_numbered_ones_by_name() {
    let registry = EnvironmentRegistry::new();
    for reference in ["Cx", "C10", "B1", "C2"] {
        let mut record = qa_record(1, "", EnvironmentStatus::Running);
        record.environment_ref = reference.to_string();
        registry.commit(record);
    }
    let listing = select(&registry, ListingScope::All, 20);
    assert_eq!(refs(&listing), ["C2", "C10", "B1", "Cx"]);
}

#[test]
fn refs_that_are_not_numbered_are_the_first_a_cap_drops_but_still_display_last() {
    let registry = EnvironmentRegistry::new();
    for reference in ["Cx", "C10", "B1", "C2"] {
        let mut record = qa_record(1, "", EnvironmentStatus::Running);
        record.environment_ref = reference.to_string();
        registry.commit(record);
    }
    let listing = select(&registry, ListingScope::All, 3);
    assert_eq!(refs(&listing), ["C2", "C10", "B1"]);
    let listing = select(&registry, ListingScope::All, 2);
    assert_eq!(refs(&listing), ["C2", "C10"]);
    let listing = select(&registry, ListingScope::All, 4);
    let ranks: Vec<(&str, usize)> = listing
        .shown
        .iter()
        .map(|row| (row.record.environment_ref.as_str(), row.keep_rank))
        .collect();
    assert_eq!(ranks, [("C2", 1), ("C10", 0), ("B1", 2), ("Cx", 3)]);
}

#[test]
fn an_empty_registry_lists_nothing_and_counts_nothing() {
    let listing = select(&EnvironmentRegistry::new(), ListingScope::Actionable, 20);
    assert!(listing.shown.is_empty());
    assert_eq!((listing.total, listing.hidden, listing.omitted), (0, 0, 0));
    assert!(listing.diagnostics.is_empty());
}

#[test]
#[should_panic(expected = "a listing shows at least one row")]
fn a_zero_limit_is_a_caller_bug() {
    select(&EnvironmentRegistry::new(), ListingScope::All, 0);
}
