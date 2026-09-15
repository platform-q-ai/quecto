use super::*;
use crate::domain::session::Session;

#[cfg(test)]
#[path = "uds_session_load_tests.rs"]
mod tests;

pub(super) async fn load_session(
    store: &dyn SessionStore,
    identity: &SessionIdentity,
    ephemeral: bool,
) -> Result<Session, String> {
    if ephemeral || identity.is_ephemeral() {
        return Ok(Session::new(identity.clone()));
    }
    store
        .load(identity)
        .await
        .map(|s| s.unwrap_or_else(|| Session::new(identity.clone())))
        .map_err(|e| e.to_string())
}
