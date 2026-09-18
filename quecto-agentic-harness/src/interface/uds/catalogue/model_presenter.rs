//! Wire rendering of a model switch (#1847): the `set_model` reply carries
//! the catalogue's structured selection verdict for the requested model
//! (#1573) and, when the switch recorded a default (#2024 S2), where it
//! landed; `NoRuntime` without a record keeps the legacy response shape
//! (no payload).

use crate::application::catalogue::dto::PersistedDefault;
use crate::application::catalogue::dto::{ModelSelectionVerdict, ModelSwitched};
use crate::domain::catalogue::UnavailableReason;

pub fn render_switch(switched: &ModelSwitched) -> Option<serde_json::Value> {
    let selection = render_verdict(&switched.plan.verdict);
    let persisted = switched.persisted.as_ref().map(render_persisted);
    if selection.is_none() && persisted.is_none() {
        return None;
    }
    let mut data = serde_json::Map::new();
    if let Some(selection) = selection {
        data.insert("selection".to_string(), selection);
    }
    if let Some(persisted) = persisted {
        data.insert("persisted".to_string(), persisted);
    }
    Some(serde_json::Value::Object(data))
}

/// The `persisted` object of a `set_model` / `set_effort` reply.
pub fn render_persisted(persisted: &PersistedDefault) -> serde_json::Value {
    serde_json::json!({
        "scope": persisted.scope.as_str(),
        "path": persisted.path.to_string_lossy(),
    })
}

pub fn render_verdict(verdict: &ModelSelectionVerdict) -> Option<serde_json::Value> {
    match verdict {
        ModelSelectionVerdict::Runnable {
            provider,
            generation,
        } => Some(serde_json::json!({
            "status": "ok",
            "provider": provider,
            "generation": generation,
        })),
        ModelSelectionVerdict::NoRuntime => None,
        ModelSelectionVerdict::Unknown { reference } => Some(serde_json::json!({
            "status": "unknown_model",
            "model": reference,
        })),
        ModelSelectionVerdict::NotRunnable { reference, reasons } => Some(serde_json::json!({
            "status": "not_runnable",
            "model": reference.qualified_id(),
            "reasons": reasons.iter().map(render_reason).collect::<Vec<_>>(),
        })),
    }
}

fn render_reason(reason: &UnavailableReason) -> String {
    match reason {
        UnavailableReason::MissingCredential => "missing-credential".to_string(),
        UnavailableReason::UnsupportedTransport { transport } => {
            format!("unsupported-transport: {}", transport.stable_id())
        }
        UnavailableReason::InvalidConfiguration(detail) => {
            format!("invalid-configuration: {detail}")
        }
        UnavailableReason::PolicyDenied(detail) => format!("policy-denied: {detail}"),
    }
}

#[cfg(test)]
#[path = "model_presenter_tests.rs"]
mod tests;
