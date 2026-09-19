use super::*;

#[test]
fn a_limit_is_brought_into_range_and_defaults() {
    assert_eq!(SearchLimit::default().get(), SearchLimit::DEFAULT);
    assert_eq!(SearchLimit::clamped(None).get(), 200);
    assert_eq!(SearchLimit::clamped(Some(0)).get(), 1);
    assert_eq!(SearchLimit::clamped(Some(1)).get(), 1);
    assert_eq!(SearchLimit::clamped(Some(37)).get(), 37);
    assert_eq!(SearchLimit::clamped(Some(500)).get(), 500);
    assert_eq!(SearchLimit::clamped(Some(501)).get(), 500);
    assert_eq!(SearchLimit::clamped(Some(u64::MAX)).get(), 500);
}

#[test]
fn an_answer_is_truncated_only_when_matches_were_left_out() {
    let mut result = SearchSessionMetadataResult {
        generation: QueryGeneration(3),
        scope: SessionListScope::Global,
        rows: Vec::new(),
        total_matches: 0,
        searched: 9,
        refused: None,
        freshness: SearchFreshness::default(),
    };
    assert!(!result.truncated());
    result.total_matches = 1;
    assert!(result.truncated());
    assert_eq!(
        SearchSessionMetadataRequest::default().generation,
        QueryGeneration(0)
    );
}
