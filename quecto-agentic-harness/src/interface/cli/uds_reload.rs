use crate::application::catalogue_refresh::{
    CatalogueRefreshStatus, RefreshCatalogueSourceUseCase,
};
use crate::domain::provider::LlmProvider;
use crate::infrastructure::catalogue_discovery::ModelsJsonCatalogueRefreshAdapter;
use crate::infrastructure::config::Config;
use crate::infrastructure::reload::ReloadResult;

use super::protocol::AgentEvent;
use super::provider_reload;
use super::uds::{DispatchCtx, emit_event_to_broadcast_or_writer};

pub(super) fn apply_provider_reload_result(
    ctx: &mut DispatchCtx<'_>,
    result: Option<ReloadResult<std::sync::Arc<dyn LlmProvider>>>,
) {
    if let Some(ReloadResult::Reloaded(provider)) = result {
        ctx.agent.swap_provider(provider);
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
    apply_provider_reload_result(ctx, result);
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
    let refresh_port = ModelsJsonCatalogueRefreshAdapter::new(&inputs.base_dir);
    let use_case = RefreshCatalogueSourceUseCase::new();
    let outcomes = if let Some(provider) = provider.filter(|p| !p.trim().is_empty()) {
        vec![use_case.refresh(&refresh_port, provider)]
    } else {
        use_case.refresh_all(&refresh_port)
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
    let any_failed = outcomes
        .iter()
        .any(|outcome| matches!(outcome.status, CatalogueRefreshStatus::Failed { .. }));
    if any_failed {
        emit_event_to_broadcast_or_writer(ctx, &AgentEvent::err(id, type_name, data.to_string()))
            .await;
    } else {
        emit_event_to_broadcast_or_writer(ctx, &AgentEvent::ok(id, type_name, Some(data))).await;
    }
    false
}
