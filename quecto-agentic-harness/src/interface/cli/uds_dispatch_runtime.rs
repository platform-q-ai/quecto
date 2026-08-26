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
    // #935/#1044: re-derive the per-model output cap AND context window so a
    // model switch re-clamps subsequent turns and the pruning budget; one
    // registry load feeds both, and set_model takes them atomically so model,
    // cap, and window can never diverge.
    let (cap, window) =
        crate::interface::catalogue_runtime::published_model_limits(ctx.base_dir, &resolved_model);
    // #1573: surface the catalogue's structured selection outcome for the
    // requested model through the UDS response. The switch itself proceeds
    // regardless (open router prefixes accept ids the catalogue cannot
    // enumerate), so runtime behaviour is unchanged — the payload only adds
    // the application-layer selection verdict for this generation.
    let selection = selection_status(ctx.base_dir, &resolved_model);
    ctx.agent.set_model(resolved_model.clone(), cap, window);
    ctx.session.set_model(resolved_model);
    // Every model switch resets the session effort to `low` (#1067): a level
    // chosen for one provider (e.g. OpenAI `xhigh`) must not silently carry
    // into another provider's vocabulary, where it would be clamped on the
    // wire while the UI still displays the stale level. Explicit `low` is
    // predictable and cost-safe; the user re-raises effort via set_effort.
    let reset_effort = crate::domain::provider::EffortLevel::Low;
    if ctx.agent.effort() != Some(reset_effort) {
        ctx.agent.set_effort(reset_effort);
        ctx.session.bump_visible_generation();
    }
    tracing::debug!(new_model = %ctx.session.model(), "UDS: model switched; effort reset to low");
    let ev = AgentEvent::ok(
        args.id.as_deref(),
        &args.type_name,
        selection.map(|selection| serde_json::json!({ "selection": selection })),
    );
    emit_event_to_broadcast_or_writer(ctx, &ev).await;
    false
}

/// The structured selection outcome for one qualified model reference against
/// the published runtime generation for `base_dir` — the application
/// selection use case's verdict rendered for the UDS wire. `None` before any
/// runtime has been composed (legacy sessions and tests without a composed
/// runtime keep the legacy response shape).
pub(super) fn selection_status(
    base_dir: &std::path::Path,
    qualified: &str,
) -> Option<serde_json::Value> {
    use crate::application::provider_runtime::SelectionError;
    use crate::domain::catalogue::UnavailableReason;

    match crate::interface::catalogue_runtime::select_model(base_dir, qualified) {
        Ok(selection) => Some(serde_json::json!({
            "status": "ok",
            "provider": selection.entry.provider.id.as_str(),
            "generation": selection.generation,
        })),
        Err(SelectionError::NoRuntime) => None,
        Err(SelectionError::UnknownModel { reference }) => Some(serde_json::json!({
            "status": "unknown_model",
            "model": reference,
        })),
        Err(SelectionError::NotRunnable { reference, reasons }) => Some(serde_json::json!({
            "status": "not_runnable",
            "model": reference.qualified_id(),
            "reasons": reasons
                .iter()
                .map(|reason| match reason {
                    UnavailableReason::MissingCredential => "missing-credential".to_string(),
                    UnavailableReason::UnsupportedTransport { transport } =>
                        format!("unsupported-transport: {}", transport.stable_id()),
                    UnavailableReason::InvalidConfiguration(detail) =>
                        format!("invalid-configuration: {detail}"),
                    UnavailableReason::PolicyDenied(detail) =>
                        format!("policy-denied: {detail}"),
                })
                .collect::<Vec<_>>(),
        })),
    }
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
    let valid =
        crate::domain::catalogue::ModelCapabilities::effort_vocabulary_for(ctx.session.model());
    let ev = match EffortLevel::parse(effort)
        .filter(|level| valid.iter().any(|v| v == level.as_str()))
    {
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
                valid.join(", ")
            ),
        ),
    };
    emit_event_to_broadcast_or_writer(ctx, &ev).await;
    false
}

#[cfg(test)]
#[path = "uds_dispatch_runtime_tests.rs"]
mod selection_status_tests;
