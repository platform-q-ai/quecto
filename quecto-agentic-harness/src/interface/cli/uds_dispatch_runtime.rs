use super::AgentEvent;
use super::{DispatchCtx, emit_event_to_broadcast_or_writer};

pub(super) struct SetModelArgs {
    pub(super) id: Option<String>,
    pub(super) type_name: String,
    pub(super) model: Option<String>,
    pub(super) provider: Option<String>,
    pub(super) model_id: Option<String>,
}

pub(super) fn resolve_set_model_target(
    model: Option<String>,
    provider: Option<String>,
    model_id: Option<String>,
) -> Result<String, &'static str> {
    if let Some(m) = model.filter(|m| !m.trim().is_empty()) {
        return Ok(m);
    }
    match (provider, model_id) {
        (Some(provider), Some(model_id)) => {
            if provider.trim().is_empty() || model_id.trim().is_empty() {
                Err("set_model requires non-empty model, or non-empty provider+modelId")
            } else {
                Ok(format!("{provider}/{model_id}"))
            }
        }
        _ => Err("set_model requires model, or provider+modelId"),
    }
}

pub(super) async fn handle_set_model(args: SetModelArgs, ctx: &mut DispatchCtx<'_>) -> bool {
    super::super::uds_reload::poll_provider_reload_for_ctx(ctx).await;
    let resolved_model = match resolve_set_model_target(args.model, args.provider, args.model_id) {
        Ok(m) => m,
        Err(msg) => {
            let ev = AgentEvent::err(args.id.as_deref(), &args.type_name, msg);
            emit_event_to_broadcast_or_writer(ctx, &ev).await;
            return false;
        }
    };
    // The change-active-model use case (#1847) republishes the catalogue,
    // applies the model with its declared limits (#935/#1044) and resets the
    // effort for the new model (#1067); the reply carries the catalogue's
    // selection verdict (#1573) — the switch itself proceeds regardless.
    let model = ctx.catalogue.model.clone();
    let switched = model.execute(ctx.agent, &resolved_model);
    ctx.session.set_model(switched.plan.model.clone());
    if switched.effort_changed {
        ctx.session.bump_visible_generation();
    }
    tracing::debug!(new_model = %ctx.session.model(), effort = ?ctx.agent.effort(), "UDS: model switched");
    let ev = AgentEvent::ok(
        args.id.as_deref(),
        &args.type_name,
        crate::interface::uds::catalogue::model_presenter::render_switch(&switched),
    );
    emit_event_to_broadcast_or_writer(ctx, &ev).await;
    false
}

/// Switch the session's reasoning effort at runtime (#1067, #1848).
///
/// The change-reasoning-effort use case validates the level against the
/// ACTIVE model's catalogue vocabulary — never a cross-provider union — so a
/// level another model accepts is rejected here, listing exactly the levels
/// this session can use. On rejection the previous setting stays in effect.
pub(super) async fn handle_set_effort(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    type_name: &str,
    effort: &str,
) -> bool {
    use crate::interface::uds::catalogue::effort_presenter;
    let request = crate::application::catalogue::dto::EffortChangeRequest {
        model: ctx.session.model().to_string(),
        level: effort.to_string(),
    };
    let before = ctx.agent.effort();
    let ev = match ctx.catalogue.effort.execute(ctx.agent, &request) {
        Ok(outcome) => {
            if before != Some(outcome.effective) {
                ctx.session.bump_visible_generation();
            }
            tracing::debug!(effort = outcome.effective.as_str(), "UDS: effort switched");
            AgentEvent::ok(
                id,
                type_name,
                Some(effort_presenter::render_change(&outcome)),
            )
        }
        Err(error) => AgentEvent::err(id, type_name, effort_presenter::render_error(&error)),
    };
    emit_event_to_broadcast_or_writer(ctx, &ev).await;
    false
}

#[cfg(test)]
#[path = "uds_dispatch_runtime_tests.rs"]
mod selection_status_tests;
