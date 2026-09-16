//! Wire rendering of a model switch (#1847): the `set_model` reply carries
//! the catalogue's structured selection verdict for the requested model
//! (#1573); `NoRuntime` keeps the legacy response shape (no payload).

use crate::application::catalogue::dto::{ModelSelectionVerdict, ModelSwitched};
use crate::domain::catalogue::UnavailableReason;

pub fn render_switch(switched: &ModelSwitched) -> Option<serde_json::Value> {
    render_verdict(&switched.plan.verdict)
        .map(|selection| serde_json::json!({ "selection": selection }))
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
