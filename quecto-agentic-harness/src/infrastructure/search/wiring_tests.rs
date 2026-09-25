use super::*;
use crate::infrastructure::config::grep_tool::GrepRelevanceConfig;

/// The configured bounds reach the judge; a missing key or a setting out of
/// range refuses ranking with the reason.
#[test]
fn ranking_is_wired_with_the_configured_bounds() {
    let mut relevance = GrepRelevanceConfig {
        enabled: true,
        concurrency: 12,
        max_candidates: 250,
        ..GrepRelevanceConfig::default()
    };
    let (judge, max_candidates) =
        ranking_judge(&relevance, Some("key".into())).expect("in range, with a key");
    assert_eq!(judge.concurrency(), 12);
    assert_eq!(max_candidates, 250);
    let missing = ranking_judge(&relevance, None).err().unwrap();
    assert!(missing.contains("no TypeSafe key was found"), "{missing}");
    relevance.concurrency = 0;
    let refused = ranking_judge(&relevance, Some("key".into())).err().unwrap();
    assert!(refused.contains("concurrency must be 1..=64"), "{refused}");
}
