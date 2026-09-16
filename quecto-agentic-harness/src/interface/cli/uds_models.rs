//! UDS catalogue commands: `list_models` (#1845, answered through the
//! composed controller) and `refresh_models` (epic #1193, slice 4). The TUI
//! model list is a projection of the `list_models` response, so CLI, UDS,
//! and TUI all render one snapshot generation.

use super::uds::DispatchCtx;

/// UDS `list_models` (#1845): the composed controller lists, the presenter
/// renders the legacy wire shape. Nothing here parses `models.json` or
/// constructs a use case.
pub(super) fn list_models_response(ctx: &DispatchCtx<'_>) -> serde_json::Value {
    crate::interface::uds::catalogue::list_models_presenter::render(
        &ctx.catalogue.list_models.list(),
    )
}

/// UDS `refresh_models` (epic #1193 slice 4, #1846): drive the composed
/// refresh use case and render per-source outcomes on the wire. The
/// dispatch loop runs this on a blocking worker thread (see
/// `dispatch_fieldless_command`), so other UDS commands stay serviced while
/// a refresh is in flight; the per-source budget is still kept tight so an
/// unattended refresh converges quickly.
pub fn refresh_models_data(
    refresh: &crate::application::catalogue::use_cases::RefreshCatalogueSources,
    source: Option<&str>,
) -> serde_json::Value {
    use crate::application::catalogue::dto::{RefreshBounds, RefreshSelection};
    let selection = match source {
        Some(name) => RefreshSelection::Only(vec![name.to_string()]),
        None => RefreshSelection::All,
    };
    let bounds = RefreshBounds {
        timeout: std::time::Duration::from_secs(4),
        ..RefreshBounds::default()
    };
    crate::interface::uds::catalogue::refresh_presenter::render(
        &refresh.execute(&selection, bounds),
    )
}

#[cfg(test)]
#[path = "uds_models_tests.rs"]
mod tests;
