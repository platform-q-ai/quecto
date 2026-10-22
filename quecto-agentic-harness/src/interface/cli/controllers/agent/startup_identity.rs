//! The identity a UDS loop opens on (#1976), and the composed builders it
//! is opened over.
use super::{AgentFlags, CliContext};
use crate::domain::sessions::entities::session_identity::SessionIdentity;

/// Composition's builders a UDS loop is opened over: the interface never
/// constructs a session store (#1970) or a session's agent-turn use cases
/// (#1909, #1855), so each missing one refuses the start with its own
/// reason.
pub(super) fn composed_loop_builders(
    ctx: &CliContext,
    stderr: &mut String,
) -> Option<(
    crate::interface::cli::SessionHandlesBuilder,
    crate::interface::cli::TurnSessionBuilder,
)> {
    let Some(sessions) = ctx.sessions else {
        stderr.push_str("agent: sessions capability not composed\n");
        return None;
    };
    let Some(turn_session) = ctx.turn_session else {
        stderr.push_str("agent: agent-turn capability not composed\n");
        return None;
    };
    Some((sessions, turn_session))
}

/// The identity a UDS loop opens on (#1976): ephemeral (never persisted),
/// the named `cli:<name>` session, or a fresh user-chat identity drawn from
/// composition's generator — the interface generates no key.
pub(super) fn resolve_startup_identity(
    ctx: &CliContext,
    flags: &AgentFlags,
    ephemeral: bool,
    stderr: &mut String,
) -> Option<SessionIdentity> {
    if ephemeral {
        return Some(SessionIdentity::ephemeral());
    }
    if let Some(name) = flags.session_name.as_deref() {
        // Admitted by the same allowlist at flag parse; a refusal here is
        // defensive.
        return match SessionIdentity::named_cli(name) {
            Ok(identity) => Some(identity),
            Err(e) => {
                stderr.push_str(&format!("{e}\n"));
                None
            }
        };
    }
    let Some(fresh_identity) = ctx.fresh_session_identity else {
        stderr.push_str("agent: fresh session identity generator not composed\n");
        return None;
    };
    Some(fresh_identity().fresh_identity())
}
