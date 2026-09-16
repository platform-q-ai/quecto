//! Integration tests for rebuildable scope catalogue (#2001 D3).

use super::*;
use crate::application::sessions::dto::scope_listing::ScopeListQuery;
use crate::application::sessions::ports::SessionScopeCatalogue;
use crate::domain::session_home_scope::{
    AssociationProvenance, CanonicalExecutionLocation, HomeScopeMetadata, SessionHomeScope,
};
use crate::domain::session::SessionSummary;
use tempfile::TempDir;

fn summary(key: &str, title: &str) -> SessionSummary {
    SessionSummary {
        key: key.into(),
        title: title.into(),
        message_count: 3,
        updated_unix_secs: Some(99),
    }
}

#[tokio::test]
async fn rebuild_from_summaries_marks_legacy_unscoped() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("catalogue.json");
    let cat = FileScopeCatalogue::new(&path);
    let report = cat
        .rebuild_from_summaries(&[summary("chat-1", "hello"), summary("cli:x", "x")])
        .unwrap();
    assert_eq!(report.rows_written, 2);
    assert!(report.rebuilt_from_scratch);
    let rows = cat
        .list(&ScopeListQuery::global(""))
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r.is_legacy_unscoped));
    // Local scope sees none of the legacy rows.
    let local = cat
        .list(&ScopeListQuery::local(SessionHomeScope::scoped(
            CanonicalExecutionLocation::from_canonical_path("/any"),
            None,
        )))
        .await
        .unwrap();
    assert!(local.is_empty());
}

#[tokio::test]
async fn corrupt_catalogue_loads_empty_then_rebuild_restores() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("catalogue.json");
    std::fs::write(&path, "{not-json").unwrap();
    let cat = FileScopeCatalogue::new(&path);
    assert!(
        cat.list(&ScopeListQuery::global(""))
            .await
            .unwrap()
            .is_empty()
    );
    cat.rebuild_from_summaries(&[summary("chat-ok", "t")])
        .unwrap();
    let rows = cat.list(&ScopeListQuery::global("chat-ok")).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert!(path.is_file());
}

#[tokio::test]
async fn rebuild_with_home_metadata_enables_local_match() {
    let tmp = TempDir::new().unwrap();
    let cat = FileScopeCatalogue::new(tmp.path().join("c.json"));
    let loc = CanonicalExecutionLocation::from_canonical_path("/proj");
    let meta = HomeScopeMetadata::v1_scoped(
        loc.clone(),
        None,
        AssociationProvenance::CreatedWithScope,
    );
    cat.rebuild_from_authority(&[(summary("chat-p", "proj work"), Some(meta))])
        .unwrap();
    let local = cat
        .list(&ScopeListQuery::local(SessionHomeScope::scoped(loc, None)))
        .await
        .unwrap();
    assert_eq!(local.len(), 1);
    assert_eq!(local[0].opaque_key(), "chat-p");
    assert!(!local[0].is_legacy_unscoped);
}

#[tokio::test]
async fn global_search_filters_metadata_fields() {
    let tmp = TempDir::new().unwrap();
    let cat = FileScopeCatalogue::new(tmp.path().join("c.json"));
    let loc = CanonicalExecutionLocation::from_canonical_path("/work/app");
    cat.rebuild(vec![scoped_row("chat-z", "Alpha title", loc, None)])
        .await
        .unwrap();
    let hits = cat.list(&ScopeListQuery::global("alpha")).await.unwrap();
    assert_eq!(hits.len(), 1);
    let miss = cat.list(&ScopeListQuery::global("zzz")).await.unwrap();
    assert!(miss.is_empty());
}

#[tokio::test]
async fn upsert_and_remove_round_trip() {
    let tmp = TempDir::new().unwrap();
    let cat = FileScopeCatalogue::new(tmp.path().join("c.json"));
    let row = scoped_row(
        "chat-u",
        "u",
        CanonicalExecutionLocation::from_canonical_path("/u"),
        None,
    );
    cat.upsert(row.clone()).await.unwrap();
    assert_eq!(cat.list(&ScopeListQuery::global("chat-u")).await.unwrap().len(), 1);
    cat.remove(&row.identity).await.unwrap();
    assert!(cat.list(&ScopeListQuery::global("chat-u")).await.unwrap().is_empty());
}
