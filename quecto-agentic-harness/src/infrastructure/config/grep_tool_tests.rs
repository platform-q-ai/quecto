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

#[test]
fn limits_in_range_are_given_and_out_of_range_refused() {
    let mut config = GrepRelevanceConfig::default();
    assert_eq!(
        config.limits(),
        Ok((30, std::time::Duration::from_secs(10)))
    );
    for (max, secs, refused) in [
        (0, 10, "max_candidates must be 1..=100 (got 0)"),
        (101, 10, "max_candidates must be 1..=100 (got 101)"),
        (30, 0, "timeout_secs must be 1..=60 (got 0)"),
        (30, 61, "timeout_secs must be 1..=60 (got 61)"),
    ] {
        config.max_candidates = max;
        config.timeout_secs = secs;
        assert!(
            config.limits().unwrap_err().contains(refused),
            "{max} {secs}"
        );
    }
    config.max_candidates = 100;
    config.timeout_secs = 60;
    assert!(config.limits().is_ok());
}
