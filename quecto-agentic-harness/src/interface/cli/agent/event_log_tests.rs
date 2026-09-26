use super::*;

/// #2150: which agents keep an audit log. A workflow session with a key
/// keeps one as before; with the event log on every agent does, keyed by its
/// session or a key of its own, except one asked to leave nothing behind.
#[test]
fn every_agent_but_an_ephemeral_one_logs_when_the_event_log_is_on() {
    assert_eq!(log_key(true, false, "s", false), Some("s".into()));
    assert_eq!(log_key(true, true, "s", false), None);
    assert_eq!(log_key(false, false, "s", false), None);
    assert_eq!(log_key(false, false, "s", true), Some("s".into()));
    assert_eq!(log_key(false, true, "s", true), None);
    assert_eq!(log_key(false, true, "", true), None);
    assert_eq!(log_key(true, false, "", false), None);
    let own = log_key(false, false, "", true).unwrap();
    assert!(
        own.starts_with(&format!("unkeyed-{}-", std::process::id())),
        "{own}"
    );
}

/// #2150 review: a one-shot session logs under the key it runs as.
#[test]
fn a_one_shot_session_logs_under_the_key_it_runs_as() {
    assert_eq!(one_shot_key(None), "cli:default");
    assert_eq!(one_shot_key(Some("foo")), "cli:foo");
}
