use super::AgentEvent;
use super::{DispatchCtx, emit_event_to_broadcast_or_writer};
use crate::infrastructure::model_registry::ModelRegistry;

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
    // #935/#1044: re-derive the per-model output cap AND context window so a
    // model switch re-clamps subsequent turns and the pruning budget; one
    // registry load feeds both, and set_model takes them atomically so model,
    // cap, and window can never diverge.
    let (cap, window) = ModelRegistry::model_limits_from_base_dir(ctx.base_dir, &resolved_model);
    ctx.agent.set_model(resolved_model.clone(), cap, window);
    ctx.session.set_model(resolved_model);
    // Every model switch resets the session effort to `low` (#1067): a level
    // chosen for one provider (e.g. OpenAI `xhigh`) must not silently carry
    // into another provider's vocabulary, where it would be clamped on the
    // wire while the UI still displays the stale level. Explicit `low` is
    // predictable and cost-safe; the user re-raises effort via set_effort.
    ctx.agent
        .set_effort(crate::domain::provider::EffortLevel::Low);
    tracing::debug!(new_model = %ctx.session.model(), "UDS: model switched; effort reset to low");
    let ev = AgentEvent::ok(args.id.as_deref(), &args.type_name, None);
    emit_event_to_broadcast_or_writer(ctx, &ev).await;
    false
}

/// Switch the session's reasoning effort at runtime (#1067).
///
/// The level is validated against the ACTIVE model's provider vocabulary
/// (OpenAI-shaped: none/low/medium/high/xhigh; Anthropic: low/medium/high/max)
/// — never the cross-provider union — so a level another provider accepts is
/// rejected here, listing exactly the levels this session can use. On
/// rejection the previous setting stays in effect.
pub(super) async fn handle_set_effort(
    ctx: &mut DispatchCtx<'_>,
    id: Option<&str>,
    type_name: &str,
    effort: &str,
) -> bool {
    use crate::domain::provider::EffortLevel;
    let valid = EffortLevel::levels_for_model(ctx.session.model());
    let ev = match EffortLevel::parse(effort).filter(|level| valid.contains(level)) {
        Some(level) => {
            if ctx.agent.effort() != Some(level) {
                ctx.agent.set_effort(level);
                ctx.session.bump_visible_generation();
            }
            tracing::debug!(effort = level.as_str(), "UDS: effort switched");
            AgentEvent::ok(
                id,
                type_name,
                Some(serde_json::json!({ "effort": level.as_str() })),
            )
        }
        None => AgentEvent::err(
            id,
            type_name,
            format!(
                "invalid effort level \"{effort}\"; valid levels: {}",
                EffortLevel::levels_list(valid)
            ),
        ),
    };
    emit_event_to_broadcast_or_writer(ctx, &ev).await;
    false
}
