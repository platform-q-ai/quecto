use crate::application::catalogue_refresh::{
    CatalogueRefreshOutcome, CatalogueRefreshStatus, RefreshCatalogueSourceUseCase,
};
use crate::application::provider_runtime::CatalogueRuntimeSnapshot;
use crate::infrastructure::catalogue_discovery::ModelsJsonCatalogueRefreshAdapter;
use crate::infrastructure::config::Config;
use crate::infrastructure::reload::ReloadResult;

use super::protocol::AgentEvent;
use super::provider_reload;
use super::uds::{DispatchCtx, emit_event_to_broadcast_or_writer};

pub(super) fn apply_provider_reload_result(
    ctx: &mut DispatchCtx<'_>,
    result: Option<ReloadResult<CatalogueRuntimeSnapshot>>,
) {
    if let Some(ReloadResult::Reloaded(runtime)) = result {
        ctx.agent.swap_runtime(runtime);
        if let Some(inputs) = ctx.provider_reload_inputs {
            match Config::load_with_env(
                inputs.config_path.to_str().unwrap_or(""),
                &inputs.env_overrides,
            ) {
                Ok(config) => {
                    let entries = config
                        .tools
                        .policy
                        .entries
                        .into_iter()
                        .map(|(stable_id, entry)| (stable_id, entry.scope))
                        .collect();
                    let unknown = ctx.agent.apply_persisted_tool_policy_entries(&entries);
                    for stable_id in unknown {
                        tracing::warn!(target: "reload", stable_id = %stable_id, "tools.policy reload entry did not match a registered tool");
                    }
                }
                Err(err) => {
                    tracing::warn!(target: "reload", error = %err, "reloaded provider but failed to reapply tools.policy");
                }
            }
        }
    }
}

/// Record a failed rebuild: the session keeps its last valid generation, learns
/// why, and retries once. The cause may lie outside the watched sources (an
/// unreadable credential store, a transient endpoint), so the fingerprints are
/// forgotten on the first failure only — a persistent failure then settles
/// instead of recomposing the runtime on every command.
fn record_reload_failure(ctx: &mut DispatchCtx<'_>, error: String) {
    if ctx.agent.catalogue_error().is_none()
        && let Some(reload) = ctx.provider_reload.as_deref_mut()
    {
        reload.invalidate_sources();
    }
    ctx.agent.set_catalogue_error(Some(error));
}

/// Ask the reload gate to rebuild again when the session is carrying a
/// catalogue error, so a fixed cause outside the watched files is picked up.
pub(super) fn retry_catalogue_error_for_ctx(ctx: &mut DispatchCtx<'_>) {
    if ctx.agent.catalogue_error().is_some()
        && let Some(reload) = ctx.provider_reload.as_deref_mut()
    {
        reload.invalidate_sources();
    }
}

pub(super) async fn poll_provider_reload_for_ctx(ctx: &mut DispatchCtx<'_>) {
    let result = provider_reload::poll_provider_reload(
        ctx.provider_reload.as_deref_mut(),
        ctx.provider_reload_inputs,
    )
    .await;
    match result {
        Some(Ok(result)) => apply_provider_reload_result(ctx, Some(result)),
        Some(Err(error)) => record_reload_failure(ctx, error),
        None => {}
    }
}

pub(super) async fn handle_reload(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    type_name: &str,
) -> bool {
    match provider_reload::force_provider_reload(
        ctx.provider_reload.as_deref_mut(),
        ctx.provider_reload_inputs,
    )
    .await
    {
        Some(Ok(result)) => {
            apply_provider_reload_result(ctx, Some(result));
            emit_event_to_broadcast_or_writer(ctx, &AgentEvent::ok(id, type_name, None)).await;
        }
        Some(Err(err)) => {
            // The published generation stays the last valid one, so anyone
            // listing models afterwards must still learn the catalogue on disk
            // is broken — and the gate retries once, so a cause outside the
            // watched files does not leave the error stuck until an unrelated
            // edit.
            record_reload_failure(ctx, err.clone());
            emit_event_to_broadcast_or_writer(ctx, &AgentEvent::err(id, type_name, err)).await;
        }
        None => {
            emit_event_to_broadcast_or_writer(
                ctx,
                &AgentEvent::err(id, type_name, "provider reload is not configured"),
            )
            .await;
        }
    }
    false
}

/// One sentence naming the sources that failed, for a client that renders the
/// error text directly.
fn refresh_failure_message(outcomes: &[CatalogueRefreshOutcome]) -> String {
    let failed: Vec<_> = outcomes
        .iter()
        .filter_map(|outcome| match &outcome.status {
            CatalogueRefreshStatus::Failed { error } => {
                Some(format!("{}: {error}", outcome.source))
            }
            _ => None,
        })
        .collect();
    format!("catalogue refresh failed for {}", failed.join("; "))
}

/// How long a `refresh_models` command may hold the dispatch loop before the
/// connection is released and the refresh continues without it. This is a UI
/// budget, not the adapter's worst case: the loop serves one command at a time,
/// so anything longer reads as a frozen session. A refresh that overruns still
/// finishes and republishes `models.json`, which the reload gate then picks up.
const DISPATCH_REFRESH_LIMIT: std::time::Duration = std::time::Duration::from_secs(8);

pub(super) async fn handle_refresh_models(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    type_name: &str,
    provider: Option<&str>,
) -> bool {
    let Some(inputs) = ctx.provider_reload_inputs else {
        emit_event_to_broadcast_or_writer(
            ctx,
            &AgentEvent::err(id, type_name, "provider reload is not configured"),
        )
        .await;
        return false;
    };
    let base_dir = inputs.base_dir.clone();
    let provider = provider
        .filter(|p| !p.trim().is_empty())
        .map(str::to_string);
    // The dispatch loop serves one command at a time, so discovery must never
    // hold it for longer than a user would wait for any other command. The
    // blocking work is bounded here rather than only inside the adapter: if it
    // overruns, the loop is released and the detached thread finishes writing
    // whatever it had already fetched.
    let refresh = tokio::task::spawn_blocking(move || {
        let refresh_port = ModelsJsonCatalogueRefreshAdapter::for_session(base_dir);
        let use_case = RefreshCatalogueSourceUseCase::new();
        if let Some(provider) = provider {
            vec![use_case.refresh(&refresh_port, &provider)]
        } else {
            use_case.refresh_all(&refresh_port)
        }
    });
    let outcomes = match tokio::time::timeout(DISPATCH_REFRESH_LIMIT, refresh).await {
        Ok(Ok(outcomes)) => outcomes,
        Ok(Err(err)) => {
            emit_event_to_broadcast_or_writer(
                ctx,
                &AgentEvent::err(
                    id,
                    type_name,
                    format!("catalogue refresh task failed: {err}"),
                ),
            )
            .await;
            return false;
        }
        Err(_) => {
            emit_event_to_broadcast_or_writer(
                ctx,
                &AgentEvent::err(
                    id,
                    type_name,
                    format!(
                        "catalogue refresh is still running after {}s; it will finish in the \
                         background — reopen the model list to see the result",
                        DISPATCH_REFRESH_LIMIT.as_secs()
                    ),
                ),
            )
            .await;
            return false;
        }
    };
    let data = serde_json::json!({
        "sources": outcomes.iter().map(|outcome| serde_json::json!({
            "source": outcome.source,
            "status": match &outcome.status {
                CatalogueRefreshStatus::Refreshed { .. } => "refreshed",
                CatalogueRefreshStatus::Skipped { .. } => "skipped",
                CatalogueRefreshStatus::Failed { .. } => "failed",
            },
            "models": match &outcome.status {
                CatalogueRefreshStatus::Refreshed { models } => Some(*models),
                _ => None,
            },
            "reason": match &outcome.status {
                CatalogueRefreshStatus::Skipped { reason } => Some(reason.as_str()),
                CatalogueRefreshStatus::Failed { error } => Some(error.as_str()),
                _ => None,
            },
        })).collect::<Vec<_>>()
    });
    let any_refreshed = outcomes
        .iter()
        .any(|outcome| matches!(outcome.status, CatalogueRefreshStatus::Refreshed { .. }));
    let any_failed = outcomes
        .iter()
        .any(|outcome| matches!(outcome.status, CatalogueRefreshStatus::Failed { .. }));
    if any_refreshed {
        match provider_reload::force_provider_reload(
            ctx.provider_reload.as_deref_mut(),
            ctx.provider_reload_inputs,
        )
        .await
        {
            Some(Ok(result)) => {
                apply_provider_reload_result(ctx, Some(result));
                // Something did refresh and the runtime swapped, so this is a
                // success that reports what failed alongside it — not a failure
                // that hides the change the user asked for.
                let event = AgentEvent::ok(id, type_name, Some(data));
                emit_event_to_broadcast_or_writer(ctx, &event).await;
            }
            // Discovery already persisted what it refreshed, so the per-source
            // outcomes are reported either way; only the follow-up reload failed.
            Some(Err(err)) => {
                emit_event_to_broadcast_or_writer(
                    ctx,
                    &AgentEvent::err(
                        id,
                        type_name,
                        format!("refreshed the catalogue but could not reload it: {err}"),
                    ),
                )
                .await;
            }
            None => {
                emit_event_to_broadcast_or_writer(
                    ctx,
                    &AgentEvent::err(
                        id,
                        type_name,
                        "refreshed the catalogue but provider reload is not configured",
                    ),
                )
                .await;
            }
        }
    } else if any_failed {
        emit_event_to_broadcast_or_writer(
            ctx,
            &AgentEvent::err(id, type_name, refresh_failure_message(&outcomes)),
        )
        .await;
    } else {
        emit_event_to_broadcast_or_writer(ctx, &AgentEvent::ok(id, type_name, Some(data))).await;
    }
    false
}
