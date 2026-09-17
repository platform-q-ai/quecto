//! Durable tool-policy persistence (#1849, #2024): writes the tool-policy
//! mutations the loop applied back into the run's config file as
//! `tools.policy.entries`, the baseline a later reload re-applies — a
//! caller of the one configuration writer, patching that one path of the
//! JSON document so every other line of the file stays as the user wrote
//! it. Composition installs it on the loop at build time.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::domain::tool::{ToolPolicyMutationStatus, ToolPolicyReconciliation};
use crate::infrastructure::config::{Config, ToolPolicyEntryConfig};

/// The loop's persistence hook shape (`application::agent_loop::ToolPolicyPersistence`).
pub type ToolPolicyPersistenceFn =
    Arc<dyn Fn(&ToolPolicyReconciliation) -> Result<(), String> + Send + Sync>;

const ENTRIES_POINTER: &str = "/tools/policy/entries";

/// A persistence hook that writes every applied mutation into
/// `config_path`. With `overlay_path`, an entry the repo-local overlay
/// already defines is refused (the overlay would shadow the persisted value
/// on the next reload) rather than written without effect.
pub fn tool_policy_persistence_for(
    config_path: PathBuf,
    overlay_path: Option<PathBuf>,
) -> ToolPolicyPersistenceFn {
    Arc::new(move |reconciliation| {
        persist_tool_policy_results(&config_path, overlay_path.as_deref(), reconciliation)
    })
}

/// Persist only user preferences already present on disk plus the applied
/// mutations: the file is read as a JSON document (never through
/// `load_with_env`, whose environment overrides may contain secrets and
/// must not be serialized back), `tools.policy.entries` is extended, the
/// result is validated as a `Config`, and only then written.
pub fn persist_tool_policy_results(
    config_path: &Path,
    overlay_path: Option<&Path>,
    reconciliation: &ToolPolicyReconciliation,
) -> Result<(), String> {
    let applied: Vec<(String, serde_json::Value)> = reconciliation
        .results
        .iter()
        .filter(|result| {
            matches!(
                result.status,
                ToolPolicyMutationStatus::Applied | ToolPolicyMutationStatus::AlreadyInState
            )
        })
        .filter_map(|result| {
            result.after.as_ref().map(|after| {
                (
                    after.stable_id.to_string(),
                    serde_json::to_value(ToolPolicyEntryConfig {
                        scope: result.requested_scope,
                    })
                    .expect("a policy entry serializes"),
                )
            })
        })
        .collect();
    if applied.is_empty() {
        return Ok(());
    }
    if let Some(overlay) = overlay_path
        && let Some(shadowed) = shadowed_by_overlay(overlay, &applied)
    {
        return Err(format!(
            "tool policy for `{shadowed}` is defined by the repo-local overlay {} and would be shadowed there; change it with `quecto config set tools.policy.entries.{shadowed} …` instead",
            overlay.display()
        ));
    }
    // The same hold `quecto config set` takes: the entries read below are
    // the ones the write replaces, whatever else is patching this file.
    let _hold = super::exclusive_hold(config_path)?;
    let mut document = match std::fs::read(config_path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| {
            format!(
                "failed to load config {} for tool policy persistence: {e}",
                config_path.display()
            )
        })?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            serde_json::Value::Object(serde_json::Map::new())
        }
        Err(error) => {
            return Err(format!(
                "failed to load config {} for tool policy persistence: {error}",
                config_path.display()
            ));
        }
    };
    let entries = entries_slot(&mut document).ok_or_else(|| {
        format!(
            "tools.policy.entries in {} is not a JSON object",
            config_path.display()
        )
    })?;
    entries.extend(applied);
    // The document must still be a configuration the loader accepts.
    Config::resolve_document(document.clone(), config_path)
        .and_then(Config::from_document)
        .map_err(|e| {
            format!(
                "refusing to persist tool policy into {}: {e}",
                config_path.display()
            )
        })?;
    super::write_document(config_path, &document)
        .map(|_| ())
        .map_err(|e| format!("failed to write config {}: {e}", config_path.display()))
}

/// The first applied stable id the overlay's `tools.policy.entries`
/// already defines, if any. An absent or unparseable overlay shadows
/// nothing (it is not applied either).
fn shadowed_by_overlay(overlay: &Path, applied: &[(String, serde_json::Value)]) -> Option<String> {
    let document: serde_json::Value = serde_json::from_slice(&std::fs::read(overlay).ok()?).ok()?;
    let entries = document.pointer(ENTRIES_POINTER)?.as_object()?;
    applied
        .iter()
        .map(|(id, _)| id)
        .find(|id| entries.contains_key(*id))
        .cloned()
}

/// `tools.policy.entries` as a mutable object, created when absent; `None`
/// when something on the way is not an object.
fn entries_slot(
    document: &mut serde_json::Value,
) -> Option<&mut serde_json::Map<String, serde_json::Value>> {
    let mut current = document;
    for segment in ENTRIES_POINTER.trim_start_matches('/').split('/') {
        current = current
            .as_object_mut()?
            .entry(segment)
            .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    }
    current.as_object_mut()
}

#[cfg(test)]
#[path = "tool_policy_tests.rs"]
mod tests;
