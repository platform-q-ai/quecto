use crate::domain::provider::LlmProvider;
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
