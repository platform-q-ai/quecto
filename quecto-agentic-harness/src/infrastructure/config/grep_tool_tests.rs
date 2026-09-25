use super::*;

#[test]
fn ranking_is_off_and_the_log_on_unless_configured() {
    let config: GrepToolConfig = serde_json::from_str("{}").unwrap();
    assert!(!config.relevance.enabled);
    assert_eq!(config.relevance.model, "jev-latest");
    // Calls cost next to nothing: every match up to a runaway guard is
    // judged, many at a time.
    assert_eq!(config.relevance.max_candidates, 1000);
    assert_eq!(config.relevance.timeout_secs, 30);
    assert_eq!(config.relevance.concurrency, 32);
    assert!(config.log.enabled);
    let config: GrepToolConfig = serde_json::from_str(
        r#"{"relevance":{"enabled":true,"max_candidates":5,"concurrency":4},"log":{"enabled":false}}"#,
    )
    .unwrap();
    assert!(config.relevance.enabled);
    assert_eq!(config.relevance.max_candidates, 5);
    assert_eq!(config.relevance.concurrency, 4);
    assert!(!config.log.enabled);
}

#[test]
fn limits_in_range_are_given_and_out_of_range_refused() {
    let mut config = GrepRelevanceConfig::default();
    assert_eq!(
        config.limits(),
        Ok(RankingLimits {
            max_candidates: 1000,
            timeout: std::time::Duration::from_secs(30),
            concurrency: 32,
        })
    );
    for (max, secs, concurrency, refused) in [
        (0, 30, 32, "max_candidates must be 1..=5000 (got 0)"),
        (5001, 30, 32, "max_candidates must be 1..=5000 (got 5001)"),
        (1000, 0, 32, "timeout_secs must be 1..=120 (got 0)"),
        (1000, 121, 32, "timeout_secs must be 1..=120 (got 121)"),
        (1000, 30, 0, "concurrency must be 1..=64 (got 0)"),
        (1000, 30, 65, "concurrency must be 1..=64 (got 65)"),
    ] {
        config.max_candidates = max;
        config.timeout_secs = secs;
        config.concurrency = concurrency;
        assert!(
            config.limits().unwrap_err().contains(refused),
            "{max} {secs} {concurrency}"
        );
    }
    config.max_candidates = 5000;
    config.timeout_secs = 120;
    config.concurrency = 64;
    assert!(config.limits().is_ok());
}
