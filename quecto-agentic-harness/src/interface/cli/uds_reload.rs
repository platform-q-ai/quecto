use crate::application::catalogue_refresh::{
    CatalogueRefreshStatus, RefreshCatalogueSourceUseCase,
};
use crate::infrastructure::catalogue_discovery::ModelsJsonCatalogueRefreshAdapter;
use crate::infrastructure::config::Config;
use crate::infrastructure::reload::ReloadResult;

use super::protocol::AgentEvent;
use super::provider_reload;
use super::uds::{DispatchCtx, emit_event_to_broadcast_or_writer};

pub(super) fn apply_provider_reload_result(
    ctx: &mut DispatchCtx<'_>,
    result: Option<ReloadResult<crate::application::catalogue_runtime::CatalogueRuntimeSnapshot>>,
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

fn refresh_outcomes_should_reload_runtime(
    outcomes: &[crate::application::catalogue_refresh::CatalogueRefreshOutcome],
) -> bool {
    outcomes
        .iter()
        .any(|outcome| matches!(outcome.status, CatalogueRefreshStatus::Refreshed { .. }))
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
    let any_failed = outcomes
        .iter()
        .any(|outcome| matches!(outcome.status, CatalogueRefreshStatus::Failed { .. }));
    let should_reload = refresh_outcomes_should_reload_runtime(&outcomes);
    if !should_reload {
        emit_event_to_broadcast_or_writer(ctx, &AgentEvent::err(id, type_name, data.to_string()))
            .await;
    } else {
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
    }
    false
}

#[cfg(test)]
mod refresh_tests {
    use super::*;
    use crate::application::catalogue_refresh::CatalogueRefreshOutcome;

    #[test]
    fn refresh_all_partial_success_triggers_reload_for_persisted_successes() {
        let outcomes = vec![
            CatalogueRefreshOutcome {
                source: "provider-a".to_string(),
                status: CatalogueRefreshStatus::Refreshed { models: 3 },
            },
            CatalogueRefreshOutcome {
                source: "provider-b".to_string(),
                status: CatalogueRefreshStatus::Failed {
                    error: "network failed".to_string(),
                },
            },
        ];

        assert!(refresh_outcomes_should_reload_runtime(&outcomes));
    }

    #[test]
    fn refresh_all_total_failure_does_not_reload_runtime() {
        let outcomes = vec![CatalogueRefreshOutcome {
            source: "provider-b".to_string(),
            status: CatalogueRefreshStatus::Failed {
                error: "network failed".to_string(),
            },
        }];

        assert!(!refresh_outcomes_should_reload_runtime(&outcomes));
    }

    #[test]
    fn refresh_all_all_skipped_does_not_reload_runtime() {
        let outcomes = vec![CatalogueRefreshOutcome {
            source: "provider-b".to_string(),
            status: CatalogueRefreshStatus::Skipped {
                reason: "not configured".to_string(),
            },
        }];

        assert!(!refresh_outcomes_should_reload_runtime(&outcomes));
    }
}
