//! The session a launched sub-agent runs as (#2192): the launcher starts it
//! with `-s <uuid>`, so it is `cli:<uuid>`. A uuid that is not a valid
//! session name names no session.
use crate::domain::agents::value_objects::child_end;
use crate::domain::error::DomainError;
use crate::domain::ids::AgentUuid;
use crate::domain::sessions::entities::session_identity::SessionIdentity;

/// The session name the launcher passes as `-s` for `child` — the one
/// source of truth for it (#1378: always the minted uuid, never the label).
pub fn child_session_name(child: &AgentUuid) -> &str {
    child.as_str()
}

/// The identity of the session `child` runs as: the child opens `-s <name>`
/// as `named_cli(name)`, so this is that same derivation.
pub fn child_session_identity(child: &AgentUuid) -> Result<SessionIdentity, DomainError> {
    SessionIdentity::named_cli(child_session_name(child))
}

/// The runtime key of the session `child` runs as, for its roster row;
/// empty for a uuid no child could run under.
pub fn child_runtime_key(child: &AgentUuid) -> String {
    child_session_identity(child)
        .map(|identity| identity.runtime_key().to_string())
        .unwrap_or_default()
}

/// The session key a saved roster records for a row of `origin` (#2192
/// review): a launched child's own session; none for any other row, whose
/// uuid is another agent's word.
pub fn roster_session_key(child: &AgentUuid, origin: child_end::ChildOrigin) -> String {
    match origin {
        child_end::ChildOrigin::Launched => child_runtime_key(child),
        child_end::ChildOrigin::Reported | child_end::ChildOrigin::Unverified => String::new(),
    }
}

#[cfg(test)]
#[path = "child_session_tests.rs"]
mod tests;
