use super::*;
use crate::application::sessions::dto::scope_listing::{
    row_matches_local_scope, row_matches_metadata_query, ScopeListQuery, ScopedSessionRow,
};
use crate::application::sessions::ports::{CatalogueRows, CatalogueUnit, SessionScopeCatalogue};
use crate::domain::session_home_scope::{CanonicalExecutionLocation, SessionHomeScope};
use crate::domain::session_identity::SessionIdentity;
use std::sync::Mutex;

struct MemCatalogue {
    rows: Mutex<Vec<ScopedSessionRow>>,
}

impl SessionScopeCatalogue for MemCatalogue {
    fn list(&self, query: &ScopeListQuery) -> CatalogueRows<'_> {
        let rows = self.rows.lock().unwrap().clone();
        let filtered = match query {
            ScopeListQuery::Local { current } => rows
                .into_iter()
                .filter(|r| row_matches_local_scope(r, current))
                .collect(),
            ScopeListQuery::Global { query } => rows
                .into_iter()
                .filter(|r| row_matches_metadata_query(r, query))
                .collect(),
        };
        Box::pin(async move { Ok(filtered) })
    }

    fn rebuild(&self, rows: Vec<ScopedSessionRow>) -> CatalogueUnit<'_> {
        Box::pin(async move {
            *self.rows.lock().unwrap() = rows;
            Ok(())
        })
    }

    fn upsert(&self, row: ScopedSessionRow) -> CatalogueUnit<'_> {
        Box::pin(async move {
            let mut g = self.rows.lock().unwrap();
            if let Some(pos) = g.iter().position(|r| r.identity == row.identity) {
                g[pos] = row;
            } else {
                g.push(row);
            }
            Ok(())
        })
    }
}

fn sample_rows() -> Vec<ScopedSessionRow> {
    let a = CanonicalExecutionLocation::from_canonical_path("/a");
    vec![
        ScopedSessionRow {
            identity: SessionIdentity::from_persisted_key("chat-local"),
            title: "local one".into(),
            message_count: 2,
            updated_unix_secs: Some(10),
            home_scope: SessionHomeScope::scoped(a.clone(), None),
            repository_label: None,
            execution_path: Some(a),
            is_legacy_unscoped: false,
        },
        ScopedSessionRow {
            identity: SessionIdentity::from_persisted_key("chat-legacy"),
            title: "legacy title".into(),
            message_count: 1,
            updated_unix_secs: Some(5),
            home_scope: SessionHomeScope::LegacyUnscoped,
            repository_label: None,
            execution_path: None,
            is_legacy_unscoped: true,
        },
    ]
}

#[tokio::test]
async fn local_listing_excludes_legacy_and_foreign_scopes() {
    let cat = Arc::new(MemCatalogue {
        rows: Mutex::new(sample_rows()),
    });
    let uc = ListScopedSessions::new(cat);
    let current = SessionHomeScope::scoped(
        CanonicalExecutionLocation::from_canonical_path("/a"),
        None,
    );
    let rows = uc.execute(&ScopeListQuery::local(current)).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].opaque_key(), "chat-local");
}

#[tokio::test]
async fn global_search_includes_legacy_by_title() {
    let cat = Arc::new(MemCatalogue {
        rows: Mutex::new(sample_rows()),
    });
    let uc = ListScopedSessions::new(cat);
    let rows = uc
        .execute(&ScopeListQuery::global("legacy"))
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert!(rows[0].is_legacy_unscoped);
}
