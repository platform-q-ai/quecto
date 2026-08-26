use super::AgentEvent;
use super::{DispatchCtx, emit_event_to_broadcast_or_writer};
use crate::application::catalogue::{
    ResolveModelSelectionUseCase, SelectionFailure, resolve_model_reference,
};
use crate::domain::catalogue::{ModelRef, UnavailableReason};

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

/// One user-facing sentence for a model reference that did not resolve. An
/// unknown provider prefix is a configuration problem; an unknown or ambiguous
/// model id is a naming problem, and the two must not read the same.
fn unresolved_model_message(
    snapshot: &crate::domain::catalogue::CatalogueSnapshot,
    model: &str,
    failure: SelectionFailure,
) -> String {
    match failure {
        SelectionFailure::AmbiguousModel { candidates } => format!(
            "model '{model}' is ambiguous; qualify it as one of: {}",
            candidates.join(", ")
        ),
        _ => match model.split_once('/') {
            // Whether the prefix is configured is a question for the snapshot,
            // not for the string: telling a user their working provider is
            // unconfigured because they mistyped a model id sends them to fix
            // the wrong thing.
            Some((provider, _)) if !snapshot_knows_provider(snapshot, provider) => {
                format!("no configured provider '{provider}'")
            }
            // The runtime routes any id under a configured prefix, so this is a
            // note about what the catalogue knows — not a claim that the switch
            // will fail.
            Some((provider, model_id)) => format!(
                "'{model_id}' is not in the catalogue for '{provider}', so its limits are unknown"
            ),
            None => format!("unknown model '{model}'"),
        },
    }
}

/// Whether the catalogue or the runtime knows this provider prefix at all.
fn snapshot_knows_provider(
    snapshot: &crate::domain::catalogue::CatalogueSnapshot,
    provider: &str,
) -> bool {
    let Ok(provider) = crate::domain::catalogue::ProviderId::new(provider.to_string()) else {
        return false;
    };
    snapshot.accepts_any_model(&provider)
        || snapshot
            .models()
            .iter()
            .any(|model| model.reference.provider() == &provider)
}

fn describe_unavailable_reasons(reasons: &[UnavailableReason]) -> String {
    if reasons.is_empty() {
        return "no reason recorded".to_string();
    }
    reasons
        .iter()
        .map(|reason| match reason {
            UnavailableReason::MissingCredential => "no credential is configured".to_string(),
            UnavailableReason::UnsupportedTransport { transport } => {
                // The wire spelling, matching `list_models`, not the Rust name.
                format!("no adapter for transport {}", transport.stable_id())
            }
            UnavailableReason::InvalidConfiguration(detail) => detail.clone(),
            UnavailableReason::PolicyDenied(detail) => format!("policy denied: {detail}"),
        })
        .collect::<Vec<_>>()
        .join("; ")
}

pub(super) async fn handle_set_model(args: SetModelArgs, ctx: &mut DispatchCtx<'_>) -> bool {
    super::super::uds_reload::poll_provider_reload_for_ctx(ctx).await;
    let request_id = args.id.clone();
    let type_name = args.type_name.clone();
    let resolved_model = match resolve_set_model_target(args.model, args.provider, args.model_id) {
        Ok(m) => m,
        Err(msg) => {
            let ev = AgentEvent::err(request_id.as_deref(), &type_name, msg);
            emit_event_to_broadcast_or_writer(ctx, &ev).await;
            return false;
        }
    };
    // Selecting a model records it and reports what the catalogue knows about
    // it; it does not refuse the switch. A client may legitimately select a
    // model before its credential exists, and refusing here would also surface
    // as a run-level failure for a sub-agent whose model was merely mistyped.
    let snapshot = ctx.agent.catalogue_store.current();
    let unavailable = describe_selection(&snapshot, &resolved_model);
    let (cap, window) = crate::application::catalogue::model_limits_in(&snapshot, &resolved_model);
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
    // The structured reason travels with the successful response so a client can
    // warn about a model that cannot currently run, rather than discovering it
    // at the next turn.
    let data = unavailable.map(|reason| serde_json::json!({ "unavailable": reason }));
    let ev = AgentEvent::ok(request_id.as_deref(), &type_name, data);
    emit_event_to_broadcast_or_writer(ctx, &ev).await;
    false
}

/// Why the selected model cannot currently run, or `None` when it can.
fn describe_selection(
    snapshot: &crate::domain::catalogue::CatalogueSnapshot,
    model: &str,
) -> Option<String> {
    // A bare name is routed by the runtime to its first provider, not resolved
    // through the catalogue, so the catalogue cannot say whether it will work —
    // and must not claim it will not.
    if ModelRef::parse_qualified(model).is_err() {
        return None;
    }
    match resolve_model_reference(snapshot, model) {
        Err(failure) => Some(unresolved_model_message(snapshot, model, failure)),
        // Resolved against the snapshot already in hand: this runs on every model
        // switch, and a discovered catalogue can hold thousands of entries.
        Ok(reference) => match ResolveModelSelectionUseCase::resolve_in(snapshot, &reference) {
            Ok(_) => None,
            Err(SelectionFailure::Unavailable { reasons }) => Some(format!(
                "model is unavailable: {}",
                describe_unavailable_reasons(&reasons)
            )),
            Err(failure) => Some(unresolved_model_message(
                snapshot,
                reference.qualified_id().as_str(),
                failure,
            )),
        },
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
