//! The identity a UDS loop opens on (#1976).
use super::{AgentFlags, CliContext};
use crate::domain::session_identity::SessionIdentity;

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
