use super::client::Event;

pub(crate) fn persist_session_response(
    event: Event,
) -> Option<(Option<String>, bool, Option<String>)> {
    match event {
        Event::Response {
            id,
            command,
            success,
            error,
            ..
        } if command == "persist_session" => Some((id, success, error)),
        _ => None,
    }
}
