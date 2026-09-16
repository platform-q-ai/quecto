//! Wire rendering of a catalogue refresh report (#1846): per-source outcomes
//! plus the republished generation when one was minted.

use crate::application::catalogue::dto::{CatalogueRefreshReport, SourceRefreshStatus};

pub fn render(report: &CatalogueRefreshReport) -> serde_json::Value {
    let outcomes: Vec<serde_json::Value> = report
        .outcomes
        .iter()
        .map(|outcome| {
            let (status, models, reason) = match &outcome.status {
                SourceRefreshStatus::Updated { models } => ("updated", Some(*models), None),
                SourceRefreshStatus::Unchanged { models } => ("unchanged", Some(*models), None),
                SourceRefreshStatus::Unsupported { reason } => {
                    ("unsupported", None, Some(reason.clone()))
                }
                SourceRefreshStatus::Failed { reason } => ("failed", None, Some(reason.clone())),
                SourceRefreshStatus::Cancelled => ("cancelled", None, None),
            };
            serde_json::json!({
                "source": outcome.source,
                "status": status,
                "models": models,
                "reason": reason,
            })
        })
        .collect();
    serde_json::json!({
        "outcomes": outcomes,
        "generation": report
            .resolved
            .as_ref()
            .map(|resolved| resolved.snapshot.generation()),
    })
}

#[cfg(test)]
#[path = "refresh_presenter_tests.rs"]
mod tests;
