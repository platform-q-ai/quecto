use crate::application::catalogue_refresh::{
    CatalogueRefreshStatus, RefreshCatalogueSourceUseCase,
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

pub(super) async fn poll_provider_reload_for_ctx(ctx: &mut DispatchCtx<'_>) {
    let result = provider_reload::poll_provider_reload(
        ctx.provider_reload.as_deref_mut(),
        ctx.provider_reload_inputs,
    )
    .await;
    match result {
        Some(Ok(result)) => apply_provider_reload_result(ctx, Some(result)),
        Some(Err(error)) => ctx.agent.set_catalogue_error(Some(error)),
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
            // is broken.
            ctx.agent.set_catalogue_error(Some(err.clone()));
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
    let outcomes = match tokio::task::spawn_blocking(move || {
        let refresh_port = ModelsJsonCatalogueRefreshAdapter::new(base_dir);
        let use_case = RefreshCatalogueSourceUseCase::new();
        if let Some(provider) = provider {
            vec![use_case.refresh(&refresh_port, &provider)]
        } else {
            use_case.refresh_all(&refresh_port)
        }
    })
    .await
    {
        Ok(outcomes) => outcomes,
        Err(err) => {
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
                let event = if any_failed {
                    AgentEvent::err(id, type_name, data.to_string())
                } else {
                    AgentEvent::ok(id, type_name, Some(data))
                };
                emit_event_to_broadcast_or_writer(ctx, &event).await;
            }
            // Discovery already persisted what it refreshed, so the per-source
            // outcomes are reported either way; only the follow-up reload failed.
            Some(Err(err)) => {
                let mut data = data.clone();
                data["reloadError"] = serde_json::Value::String(err);
                emit_event_to_broadcast_or_writer(
                    ctx,
                    &AgentEvent::err(id, type_name, data.to_string()),
                )
                .await;
            }
            None => {
                let mut data = data.clone();
                data["reloadError"] =
                    serde_json::Value::String("provider reload is not configured".to_string());
                emit_event_to_broadcast_or_writer(
                    ctx,
                    &AgentEvent::err(id, type_name, data.to_string()),
                )
                .await;
            }
        }
    } else if any_failed {
        emit_event_to_broadcast_or_writer(ctx, &AgentEvent::err(id, type_name, data.to_string()))
            .await;
    } else {
        emit_event_to_broadcast_or_writer(ctx, &AgentEvent::ok(id, type_name, Some(data))).await;
    }
    false
}
