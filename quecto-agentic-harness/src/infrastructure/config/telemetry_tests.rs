use super::*;

/// #2150: the event log is off unless configured; `enabled` switches it on.
#[test]
fn the_event_log_is_off_unless_switched_on() {
    let config: TelemetryConfig = serde_json::from_str("{}").unwrap();
    assert!(!config.event_log.enabled);
    let config: TelemetryConfig =
        serde_json::from_str(r#"{"event_log":{"enabled":true}}"#).unwrap();
    assert!(config.event_log.enabled);
    let whole: crate::infrastructure::config::Config =
        serde_json::from_str(r#"{"telemetry":{"event_log":{"enabled":true}}}"#).unwrap();
    assert!(whole.telemetry.event_log.enabled);
}

/// #2150: the owner's global file switches the event log on for every
/// agent, whatever config it was started with; no file or an unreadable one
/// switches nothing on.
#[test]
fn the_global_file_switches_every_agent_on() {
    let base = tempfile::tempdir().unwrap();
    assert!(!globally_enabled(base.path()));
    std::fs::write(base.path().join("config.json"), "not json").unwrap();
    assert!(!globally_enabled(base.path()));
    std::fs::write(
        base.path().join("config.json"),
        r#"{"telemetry":{"event_log":{"enabled":false}}}"#,
    )
    .unwrap();
    assert!(!globally_enabled(base.path()));
    std::fs::write(
        base.path().join("config.json"),
        r#"{"telemetry":{"event_log":{"enabled":true}}}"#,
    )
    .unwrap();
    assert!(globally_enabled(base.path()));
}
