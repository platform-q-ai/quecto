use super::*;
use crate::domain::session_scope::{
    AssociationProvenance, CanonicalExecutionLocation, RepositoryGrouping, SessionHomeScope,
    SessionScopeMetadata,
};

fn scoped(path: &str, common: Option<&str>) -> SessionScopeMetadata {
    let grouping =
        common.map(|common| RepositoryGrouping::new(format!("{path}/.git"), common).unwrap());
    SessionScopeMetadata::current(SessionHomeScope::scoped(
        CanonicalExecutionLocation::new(path).unwrap(),
        grouping,
        AssociationProvenance::Discovered,
    ))
}
fn row(key: &str, title: &str, scope: Option<SessionScopeMetadata>) -> CatalogueEntry {
    CatalogueEntry {
        key: key.into(),
        title: title.into(),
        message_count: 1,
        updated_unix_secs: Some(1),
        scope,
    }
}

#[test]
fn local_listing_groups_related_worktrees_and_excludes_legacy() {
    let catalogue = SessionCatalogue::rebuild(vec![
        row("a", "one", Some(scoped("/repo", Some("/repo/.git")))),
        row("b", "two", Some(scoped("/worktree", Some("/repo/.git")))),
        row("c", "other", Some(scoped("/other", None))),
        row("legacy", "old", None),
    ]);
    let local = catalogue.list_local(&scoped("/worktree", Some("/repo/.git")));
    assert_eq!(
        local.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(),
        vec!["a", "b"]
    );
}

#[test]
fn non_git_local_listing_uses_exact_canonical_location() {
    let catalogue = SessionCatalogue::rebuild(vec![
        row("a", "one", Some(scoped("/x", None))),
        row("b", "two", Some(scoped("/xy", None))),
    ]);
    assert_eq!(catalogue.list_local(&scoped("/x", None))[0].key, "a");
}

#[test]
fn global_search_matches_metadata_case_insensitively_and_keeps_legacy() {
    let catalogue = SessionCatalogue::rebuild(vec![
        row(
            "cli:ABC",
            "Release Notes",
            Some(scoped("/Projects/Repo", Some("/Projects/Repo/.git"))),
        ),
        row("legacy", "Old", None),
    ]);
    assert_eq!(catalogue.search_global("release").len(), 1);
    assert_eq!(catalogue.search_global("projects").len(), 1);
    assert_eq!(catalogue.search_global("abc").len(), 1);
    assert_eq!(catalogue.search_global("").len(), 2);
}

#[test]
fn rebuild_deduplicates_stale_rows_by_key_using_newest() {
    let mut old = row("same", "old", None);
    old.updated_unix_secs = Some(1);
    let mut new = row("same", "new", None);
    new.updated_unix_secs = Some(2);
    let catalogue = SessionCatalogue::rebuild(vec![old, new]);
    assert_eq!(catalogue.search_global("")[0].title, "new");
}
