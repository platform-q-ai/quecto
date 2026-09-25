use super::*;

#[test]
fn ranking_is_off_and_the_log_on_unless_configured() {
    let config: GrepToolConfig = serde_json::from_str("{}").unwrap();
    assert!(!config.relevance.enabled);
    assert_eq!(config.relevance.model, "jev-latest");
    assert_eq!(config.relevance.max_candidates, 30);
    assert_eq!(config.relevance.timeout_secs, 10);
    assert!(config.log.enabled);
    let config: GrepToolConfig = serde_json::from_str(
        r#"{"relevance":{"enabled":true,"max_candidates":5},"log":{"enabled":false}}"#,
    )
    .unwrap();
    assert!(config.relevance.enabled);
    assert_eq!(config.relevance.max_candidates, 5);
    assert!(!config.log.enabled);
}
