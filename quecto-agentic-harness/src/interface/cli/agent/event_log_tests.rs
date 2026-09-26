use super::*;

/// #2150: which agents keep an audit log. A workflow session with a key
/// keeps one as before; with the event log on every agent does, keyed by
/// its session, or its process when it has none.
#[test]
fn every_agent_logs_when_the_event_log_is_on() {
    assert_eq!(log_key(true, false, "s", false), Some("s".into()));
    assert_eq!(log_key(true, true, "s", false), None);
    assert_eq!(log_key(false, false, "s", false), None);
    assert_eq!(log_key(false, true, "s", true), Some("s".into()));
    assert_eq!(
        log_key(false, true, "", true),
        Some(format!("pid-{}", std::process::id()))
    );
    assert_eq!(
        log_key(true, false, "", true),
        Some(format!("pid-{}", std::process::id()))
    );
    assert_eq!(log_key(true, false, "", false), None);
}
