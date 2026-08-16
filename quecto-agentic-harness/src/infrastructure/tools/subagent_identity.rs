pub fn parent_identity_from_session_key(session_key: &str) -> Option<&str> {
    if session_key.is_empty() {
        return None;
    }
    Some(
        if session_key.starts_with(crate::domain::session::USER_CHAT_PREFIX) {
            session_key
        } else {
            session_key.strip_prefix("cli:").unwrap_or_else(|| {
                session_key
                    .rsplit_once(':')
                    .map_or(session_key, |(_, name)| name)
            })
        },
    )
}
