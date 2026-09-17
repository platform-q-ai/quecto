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
/// Per-source budget of a `refresh_models` request: a client waits on the
/// reply, so one slow provider must not hold the loop for the default 30 s.
pub(super) const UDS_REFRESH_BOUNDS: crate::application::catalogue::dto::RefreshBounds =
    crate::application::catalogue::dto::RefreshBounds {
        timeout: std::time::Duration::from_secs(4),
        max_response_bytes:
            crate::application::catalogue::dto::RefreshBounds::DEFAULT_MAX_RESPONSE_BYTES,
    };

pub fn refresh_models_data(
    refresh: &crate::application::catalogue::use_cases::RefreshCatalogueSources,
    source: Option<&str>,
) -> serde_json::Value {
    use crate::application::catalogue::dto::RefreshSelection;
    let selection = match source {
        Some(name) => RefreshSelection::Only(vec![name.to_string()]),
        None => RefreshSelection::All,
    };
    crate::interface::uds::catalogue::refresh_presenter::render(
        &refresh.execute(&selection, UDS_REFRESH_BOUNDS),
    )
}

#[cfg(test)]
#[path = "uds_models_tests.rs"]
mod tests;
