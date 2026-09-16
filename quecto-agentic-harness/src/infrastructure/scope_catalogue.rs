//! Derived rebuildable session-scope metadata catalogue adapter (#2001 D3).
//!
//! File-backed JSON index. Transcripts remain recovery authority; corruption
//! yields an empty in-memory view and an explicit rebuild path.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::application::sessions::dto::scope_listing::{
    row_matches_local_scope, row_matches_metadata_query, ScopeListQuery, ScopedSessionRow,
};
use crate::application::sessions::ports::{
    CatalogueRebuildReport, CatalogueRows, CatalogueUnit, SessionScopeCatalogue,
};
use crate::domain::error::DomainError;
use crate::domain::session::SessionSummary;
use crate::domain::session_home_scope::{
    CanonicalExecutionLocation, HomeScopeMetadata, RepositoryLabel, SessionHomeScope,
};
use crate::domain::session_identity::SessionIdentity;
use crate::infrastructure::atomic_write::atomic_write;

/// On-disk wire shape (camelCase) for catalogue rows.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogueFile {
    schema_version: u32,
    rows: Vec<CatalogueRowWire>,
}

const CATALOGUE_SCHEMA_V1: u32 = 1;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogueRowWire {
    key: String,
    title: String,
    message_count: usize,
    #[serde(default)]
    updated_unix_secs: Option<u64>,
    #[serde(default)]
    is_legacy_unscoped: bool,
    #[serde(default)]
    execution_path: Option<String>,
    #[serde(default)]
    repository_label: Option<String>,
    /// Optional embedded home-scope metadata JSON object.
    #[serde(default)]
    home_scope_metadata: Option<serde_json::Value>,
}

/// File-backed derived catalogue. Rebuildable; discardable on corruption.
pub struct FileScopeCatalogue {
    path: PathBuf,
    rows: Mutex<Vec<ScopedSessionRow>>,
    last_report: Mutex<Option<CatalogueRebuildReport>>,
}

impl FileScopeCatalogue {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let rows = load_rows_from_disk(&path).unwrap_or_default();
        Self {
            path,
            rows: Mutex::new(rows),
            last_report: Mutex::new(None),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Rebuild catalogue from session summaries without scope metadata
    /// (legacy-unscoped rows).
    pub fn rebuild_from_summaries(
        &self,
        summaries: &[SessionSummary],
    ) -> Result<CatalogueRebuildReport, DomainError> {
        let entries: Vec<(SessionSummary, Option<HomeScopeMetadata>)> = summaries
            .iter()
            .cloned()
            .map(|s| (s, None))
            .collect();
        self.rebuild_from_authority(&entries)
    }

    /// Rebuild from authoritative summary + optional home-scope metadata pairs.
    pub fn rebuild_from_authority(
        &self,
        entries: &[(SessionSummary, Option<HomeScopeMetadata>)],
    ) -> Result<CatalogueRebuildReport, DomainError> {
        let mut rows = Vec::with_capacity(entries.len());
        let mut skipped = 0usize;
        let mut notes = Vec::new();
        for (summary, meta) in entries {
            match row_from_authority(summary, meta.as_ref()) {
                Ok(row) => rows.push(row),
                Err(e) => {
                    skipped += 1;
                    notes.push(format!("skipped {}: {e}", summary.key));
                }
            }
        }
        let written = rows.len();
        self.persist_rows(&rows)?;
        let report = CatalogueRebuildReport {
            rows_written: written,
            rows_skipped: skipped,
            rebuilt_from_scratch: true,
            notes,
        };
        *self.last_report.lock().unwrap() = Some(report.clone());
        Ok(report)
    }

    fn persist_rows(&self, rows: &[ScopedSessionRow]) -> Result<(), DomainError> {
        let wire = CatalogueFile {
            schema_version: CATALOGUE_SCHEMA_V1,
            rows: rows.iter().map(row_to_wire).collect(),
        };
        let bytes = serde_json::to_vec_pretty(&wire).map_err(|e| {
            DomainError::Session(format!("catalogue serialize: {e}"))
        })?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                DomainError::Session(format!("catalogue parent dir: {e}"))
            })?;
        }
        atomic_write(&self.path, &bytes, None).map_err(|e| {
            DomainError::Session(format!("catalogue atomic write: {e}"))
        })?;
        *self.rows.lock().unwrap() = rows.to_vec();
        Ok(())
    }

    fn filter_rows(&self, query: &ScopeListQuery) -> Vec<ScopedSessionRow> {
        let rows = self.rows.lock().unwrap().clone();
        match query {
            ScopeListQuery::Local { current } => rows
                .into_iter()
                .filter(|r| row_matches_local_scope(r, current))
                .collect(),
            ScopeListQuery::Global { query } => rows
                .into_iter()
                .filter(|r| row_matches_metadata_query(r, query))
                .collect(),
        }
    }
}

impl SessionScopeCatalogue for FileScopeCatalogue {
    fn list(&self, query: &ScopeListQuery) -> CatalogueRows<'_> {
        let filtered = self.filter_rows(query);
        Box::pin(async move { Ok(filtered) })
    }

    fn rebuild(&self, rows: Vec<ScopedSessionRow>) -> CatalogueUnit<'_> {
        Box::pin(async move {
            let written = rows.len();
            self.persist_rows(&rows)?;
            *self.last_report.lock().unwrap() = Some(CatalogueRebuildReport {
                rows_written: written,
                rows_skipped: 0,
                rebuilt_from_scratch: true,
                notes: Vec::new(),
            });
            Ok(())
        })
    }

    fn upsert(&self, row: ScopedSessionRow) -> CatalogueUnit<'_> {
        Box::pin(async move {
            let mut rows = self.rows.lock().unwrap().clone();
            if let Some(pos) = rows.iter().position(|r| r.identity == row.identity) {
                rows[pos] = row;
            } else {
                rows.push(row);
            }
            self.persist_rows(&rows)
        })
    }

    fn remove(&self, identity: &SessionIdentity) -> CatalogueUnit<'_> {
        let key = identity.clone();
        Box::pin(async move {
            let mut rows = self.rows.lock().unwrap().clone();
            rows.retain(|r| r.identity != key);
            self.persist_rows(&rows)
        })
    }

    fn last_rebuild_report(&self) -> Option<CatalogueRebuildReport> {
        self.last_report.lock().unwrap().clone()
    }
}

fn load_rows_from_disk(path: &Path) -> Result<Vec<ScopedSessionRow>, DomainError> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let bytes = std::fs::read(path).map_err(|e| {
        DomainError::Session(format!("catalogue read: {e}"))
    })?;
    let file: CatalogueFile = serde_json::from_slice(&bytes).map_err(|e| {
        DomainError::Session(format!("catalogue corrupt/unreadable: {e}"))
    })?;
    if file.schema_version != CATALOGUE_SCHEMA_V1 {
        return Err(DomainError::Session(format!(
            "unsupported catalogue schema version {}",
            file.schema_version
        )));
    }
    let mut rows = Vec::new();
    for w in file.rows {
        match row_from_wire(w) {
            Ok(r) => rows.push(r),
            Err(_) => {
                // Skip bad records; caller may rebuild from authority.
            }
        }
    }
    Ok(rows)
}

fn row_from_authority(
    summary: &SessionSummary,
    meta: Option<&HomeScopeMetadata>,
) -> Result<ScopedSessionRow, DomainError> {
    let identity = SessionIdentity::from_persisted_key(&summary.key);
    match meta {
        Some(m) if !m.is_legacy_unscoped() => {
            let scope = m.scope().clone();
            let (repo, path) = match &scope {
                SessionHomeScope::Scoped {
                    execution_location,
                    repository_grouping,
                } => (
                    repository_grouping
                        .as_ref()
                        .map(|g| g.repository_label().clone()),
                    Some(execution_location.clone()),
                ),
                SessionHomeScope::LegacyUnscoped => (None, None),
            };
            Ok(ScopedSessionRow {
                identity,
                title: summary.title.clone(),
                message_count: summary.message_count,
                updated_unix_secs: summary.updated_unix_secs,
                home_scope: scope,
                repository_label: repo,
                execution_path: path,
                is_legacy_unscoped: false,
            })
        }
        _ => Ok(ScopedSessionRow {
            identity,
            title: summary.title.clone(),
            message_count: summary.message_count,
            updated_unix_secs: summary.updated_unix_secs,
            home_scope: SessionHomeScope::LegacyUnscoped,
            repository_label: None,
            execution_path: None,
            is_legacy_unscoped: true,
        }),
    }
}

fn row_to_wire(row: &ScopedSessionRow) -> CatalogueRowWire {
    let home_scope_metadata = if row.is_legacy_unscoped {
        None
    } else {
        // Persist enough to rebuild scoped home without guessing.
        match &row.home_scope {
            SessionHomeScope::Scoped {
                execution_location,
                repository_grouping,
            } => {
                let mut obj = serde_json::json!({
                    "schemaVersion": 1,
                    "executionLocation": execution_location.as_str(),
                });
                if let Some(g) = repository_grouping {
                    obj["repositoryLabel"] = serde_json::json!(g.repository_label().as_str());
                    obj["memberLocations"] = serde_json::json!(
                        g.member_locations()
                            .iter()
                            .map(|l| l.as_str().to_string())
                            .collect::<Vec<_>>()
                    );
                }
                if let Some(p) = row.repository_label.as_ref() {
                    obj["repositoryLabel"] = serde_json::json!(p.as_str());
                }
                Some(obj)
            }
            SessionHomeScope::LegacyUnscoped => None,
        }
    };
    CatalogueRowWire {
        key: row.opaque_key().to_string(),
        title: row.title.clone(),
        message_count: row.message_count,
        updated_unix_secs: row.updated_unix_secs,
        is_legacy_unscoped: row.is_legacy_unscoped,
        execution_path: row.execution_path.as_ref().map(|p| p.as_str().to_string()),
        repository_label: row.repository_label.as_ref().map(|l| l.as_str().to_string()),
        home_scope_metadata,
    }
}

fn row_from_wire(w: CatalogueRowWire) -> Result<ScopedSessionRow, DomainError> {
    let identity = SessionIdentity::from_persisted_key(&w.key);
    if w.is_legacy_unscoped || w.execution_path.is_none() {
        return Ok(ScopedSessionRow {
            identity,
            title: w.title,
            message_count: w.message_count,
            updated_unix_secs: w.updated_unix_secs,
            home_scope: SessionHomeScope::LegacyUnscoped,
            repository_label: None,
            execution_path: None,
            is_legacy_unscoped: true,
        });
    }
    let execution = CanonicalExecutionLocation::try_from_canonical_path(
        w.execution_path.clone().unwrap_or_default(),
    )?;
    let label = match w.repository_label.as_deref() {
        Some(s) => Some(RepositoryLabel::new(s)?),
        None => None,
    };
    let grouping = label.as_ref().map(|l| {
        crate::domain::session_home_scope::RepositoryWorktreeGrouping::related_worktrees(
            l.clone(),
            execution.clone(),
            vec![execution.clone()],
        )
    });
    Ok(ScopedSessionRow {
        identity,
        title: w.title,
        message_count: w.message_count,
        updated_unix_secs: w.updated_unix_secs,
        home_scope: SessionHomeScope::scoped(execution.clone(), grouping),
        repository_label: label,
        execution_path: Some(execution),
        is_legacy_unscoped: false,
    })
}

/// Helper for tests/adapters: build a scoped catalogue row.
pub fn scoped_row(
    key: &str,
    title: &str,
    execution: CanonicalExecutionLocation,
    label: Option<RepositoryLabel>,
) -> ScopedSessionRow {
    let grouping = label.as_ref().map(|l| {
        crate::domain::session_home_scope::RepositoryWorktreeGrouping::related_worktrees(
            l.clone(),
            execution.clone(),
            vec![execution.clone()],
        )
    });
    ScopedSessionRow {
        identity: SessionIdentity::from_persisted_key(key),
        title: title.into(),
        message_count: 0,
        updated_unix_secs: None,
        home_scope: SessionHomeScope::scoped(execution.clone(), grouping),
        repository_label: label,
        execution_path: Some(execution),
        is_legacy_unscoped: false,
    }
}

#[cfg(test)]
#[path = "scope_catalogue_tests.rs"]
mod tests;
