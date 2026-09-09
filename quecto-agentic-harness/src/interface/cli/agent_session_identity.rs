pub(super) fn resolve_uds_session_key(ephemeral: bool, session_name: Option<&str>) -> String {
    if ephemeral {
        String::new()
    } else if let Some(name) = session_name {
        crate::domain::session::Session::build_key("cli", name)
    } else {
        crate::interface::shared::generate_chat_key()
    }
}
