// Tests for infrastructure config — split out to keep config.rs under 750 lines.
use super::*;

// ===================================================================
// Issue #193: TelegramConfig default_send_to
// ===================================================================

#[test]
fn test_telegram_default_send_to_absent_when_not_set() {
    let config: Config = serde_json::from_str("{}").unwrap();
    assert!(
        config.channels.telegram.default_send_to.is_none(),
        "default_send_to should be None when not configured"
    );
}

#[test]
fn test_telegram_default_send_to_loaded_from_config() {
    let json = r#"{
        "channels": {
            "telegram": {
                "enabled": true,
                "token": "bot-token",
                "default_send_to": "telegram:123456789"
            }
        }
    }"#;
    let config: Config = serde_json::from_str(json).unwrap();
    assert_eq!(
        config.channels.telegram.default_send_to.as_deref(),
        Some("telegram:123456789")
    );
}

#[test]
fn test_telegram_config_is_backward_compatible_without_default_send_to() {
    // Existing config without default_send_to should still load fine
    let json = r#"{
        "channels": {
            "telegram": {
                "enabled": true,
                "token": "existing-token",
                "allow_from": ["12345"]
            }
        }
    }"#;
    let config: Config = serde_json::from_str(json).unwrap();
    assert!(config.channels.telegram.enabled);
    assert_eq!(config.channels.telegram.token, "existing-token");
    assert!(config.channels.telegram.default_send_to.is_none());
}
