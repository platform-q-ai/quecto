use super::*;

pub(super) async fn load_session(
    store: &dyn SessionStore,
    key: &str,
    ephemeral: bool,
) -> Result<Session, String> {
    if ephemeral || key.is_empty() {
        return Ok(Session::new(key));
    }
    store
        .load(key)
        .await
        .map(|s| s.unwrap_or_else(|| Session::new(key)))
        .map_err(|e| e.to_string())
}
