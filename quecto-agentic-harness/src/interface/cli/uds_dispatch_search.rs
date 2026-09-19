//! Edge of the `search_session_metadata` command (#2010): maps the wire
//! fields onto the application's request, asks the injected discovery handle
//! and presents the typed answer. Rows are listing rows (same fields, same
//! `homeVersion`) plus what matched; untrusted text is made safe. Nothing is
//! matched, ordered, scoped or restored here.
use super::super::uds_dispatch_query::{freshened, listed_row_json, safe_display};
use super::{AgentEvent, DispatchCtx};
use crate::application::sessions::dto::{
    SearchSessionMetadataRequest, SearchSessionMetadataResult,
};
use crate::domain::session_metadata_search::MetadataQuery;
use crate::interface::cli::protocol::SessionListScopeCommand;

pub(super) async fn handle(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    type_name: &str,
    fields: SearchFields,
) -> bool {
    let scope = fields.scope;
    let (request, malformed) = request_of(fields);
    // Not a number (R1-H5): refused in a correlated answer, nothing searched.
    let answer = match malformed {
        Some(refusal) => Ok(SearchSessionMetadataResult::refused(&request, refusal)),
        None => ctx.discovery.search(&request).await,
    };
    let event = match answer {
        Ok(result) => {
            let data = search_json(&result, scope, &request);
            AgentEvent::ok(id, type_name, Some(data))
        }
        Err(error) => AgentEvent::err(id, type_name, safe_display(&error.to_string())),
    };
    super::super::emit_response_or_frame_limit_error(ctx, id, type_name, event).await;
    false
}

/// The answer: the echoed query (the visible text, bounded), scope and generation, the
/// matched rows, how many matched and were searched, and the freshness.
pub(super) fn search_json(
    result: &SearchSessionMetadataResult,
    scope: SessionListScopeCommand,
    request: &SearchSessionMetadataRequest,
) -> serde_json::Value {
    debug_assert_eq!(result.generation, request.generation);
    let sessions: Vec<_> = result
        .rows
        .iter()
        .map(|row| {
            let mut value = listed_row_json(&row.session);
            value["repositoryLabel"] =
                serde_json::json!(row.repository_label.as_deref().map(safe_display));
            value["matched"] =
                serde_json::json!(row.matched.iter().map(|f| f.name()).collect::<Vec<_>>());
            value
        })
        .collect();
    let body = serde_json::json!({
        "query": safe_display(&MetadataQuery::shown(&request.query)),
        "scope": scope,
        "generation": result.generation.0,
        "limit": request.limit.get(),
        "sessions": sessions,
        "totalMatches": result.total_matches,
        "searched": result.searched,
        "truncated": result.truncated(),
        "refused": result.refused.as_ref().map(ToString::to_string),
    });
    let freshness = &result.freshness;
    freshened(body, &freshness.diagnostics, freshness.rebuilt)
}

#[path = "uds_search_numbers.rs"]
mod uds_search_numbers;
pub(super) use uds_search_numbers::SearchFields;
use uds_search_numbers::request_of;

#[cfg(test)]
#[path = "uds_dispatch_search_tests.rs"]
mod tests;
