//! Edge of the `search_session_metadata` command (#2010): maps the wire
//! fields onto the application's request, asks the injected discovery handle
//! and presents the typed answer. Rows are listing rows (same fields, same
//! `homeVersion`) plus what matched; untrusted text is made safe. Nothing is
//! matched, ordered, scoped or restored here.
use super::super::uds_dispatch_query::{listed_row_json, safe_display};
use super::{AgentEvent, DispatchCtx};
use crate::application::sessions::dto::{
    QueryGeneration, SearchLimit, SearchSessionMetadataRequest, SearchSessionMetadataResult,
};
use crate::interface::cli::protocol::SessionListScopeCommand;

/// The wire fields of one search, as the protocol decoded them.
pub(super) struct SearchFields {
    pub(super) query: String,
    pub(super) scope: SessionListScopeCommand,
    pub(super) generation: u64,
    pub(super) limit: Option<u64>,
}

pub(super) async fn handle(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    type_name: &str,
    fields: SearchFields,
) -> bool {
    let request = SearchSessionMetadataRequest {
        query: fields.query,
        scope: fields.scope.into(),
        generation: QueryGeneration(fields.generation),
        limit: SearchLimit::clamped(fields.limit),
    };
    let event = match ctx.list_sessions.search(&request).await {
        Ok(result) => {
            let data = search_json(&result, fields.scope, &request);
            AgentEvent::ok(id, type_name, Some(data))
        }
        Err(error) => AgentEvent::err(id, type_name, safe_display(&error.to_string())),
    };
    super::super::emit_response_or_frame_limit_error(ctx, id, type_name, event).await;
    false
}

/// The answer: the echoed query (safe, bounded), scope and generation, the
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
    let safe = |lines: &[String]| lines.iter().map(|s| safe_display(s)).collect::<Vec<_>>();
    serde_json::json!({
        "query": safe_display(&request.query.chars().take(256).collect::<String>()),
        "scope": scope,
        "generation": result.generation.0,
        "limit": request.limit.get(),
        "sessions": sessions,
        "totalMatches": result.total_matches,
        "searched": result.searched,
        "truncated": result.truncated(),
        "refused": result.refused.as_ref().map(ToString::to_string),
        "diagnostics": safe(&result.freshness.diagnostics),
        "rebuilt": result.freshness.rebuilt,
    })
}

#[cfg(test)]
#[path = "uds_dispatch_search_tests.rs"]
mod tests;
