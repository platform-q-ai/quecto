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
