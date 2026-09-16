//! Pure application projection for rebuildable session discovery metadata.
use crate::domain::session_scope::{SessionHomeScope, SessionScopeMetadata};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogueEntry {
    pub key: String,
    pub title: String,
    pub message_count: usize,
    pub updated_unix_secs: Option<u64>,
    /// Absence is the migration-safe representation of a legacy unscoped record.
    pub scope: Option<SessionScopeMetadata>,
}

#[derive(Debug, Clone, Default)]
pub struct SessionCatalogue {
    entries: Vec<CatalogueEntry>,
}

impl SessionCatalogue {
    /// Rebuild from authoritative global records. Duplicate/stale projections
    /// are collapsed deterministically to the newest record.
    pub fn rebuild(entries: impl IntoIterator<Item = CatalogueEntry>) -> Self {
        let mut by_key: HashMap<String, CatalogueEntry> = HashMap::new();
        for entry in entries {
            let replace = by_key
                .get(&entry.key)
                .is_none_or(|old| entry.updated_unix_secs >= old.updated_unix_secs);
            if replace {
                by_key.insert(entry.key.clone(), entry);
            }
        }
        let mut entries: Vec<_> = by_key.into_values().collect();
        entries.sort_by(|a, b| {
            b.updated_unix_secs
                .cmp(&a.updated_unix_secs)
                .then_with(|| a.key.cmp(&b.key))
        });
        Self { entries }
    }

    pub fn list_local(&self, current: &SessionScopeMetadata) -> Vec<&CatalogueEntry> {
        self.entries
            .iter()
            .filter(|entry| {
                entry
                    .scope
                    .as_ref()
                    .is_some_and(|scope| same_local_scope(scope, current))
            })
            .collect()
    }

    /// MVP global metadata search: title, opaque key, repository facts and execution path.
    pub fn search_global(&self, query: &str) -> Vec<&CatalogueEntry> {
        let needle = query.trim().to_lowercase();
        self.entries
            .iter()
            .filter(|entry| needle.is_empty() || searchable(entry).to_lowercase().contains(&needle))
            .collect()
    }
}

fn same_local_scope(left: &SessionScopeMetadata, right: &SessionScopeMetadata) -> bool {
    match (left.home(), right.home()) {
        (
            SessionHomeScope::Scoped {
                execution_location: l,
                repository_grouping: lg,
                ..
            },
            SessionHomeScope::Scoped {
                execution_location: r,
                repository_grouping: rg,
                ..
            },
        ) => match (lg, rg) {
            (Some(lg), Some(rg)) => lg.common_git_dir() == rg.common_git_dir(),
            (None, None) => l == r,
            _ => false,
        },
        _ => false,
    }
}
fn searchable(entry: &CatalogueEntry) -> String {
    let mut value = format!("{} {}", entry.key, entry.title);
    if let Some(scope) = &entry.scope
        && let SessionHomeScope::Scoped {
            execution_location,
            repository_grouping,
            ..
        } = scope.home()
    {
        value.push(' ');
        value.push_str(execution_location.as_str());
        if let Some(repo) = repository_grouping {
            value.push(' ');
            value.push_str(repo.worktree_git_dir());
            value.push(' ');
            value.push_str(repo.common_git_dir());
        }
    }
    value
}

#[cfg(test)]
#[path = "catalogue_tests.rs"]
mod tests;
