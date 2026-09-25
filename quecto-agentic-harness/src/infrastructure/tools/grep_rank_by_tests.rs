//! `rank_by` and the search log through the tool itself (#2136 slice B),
//! over the real rg.
use super::*;
use crate::application::search::ports::{PortFuture, RankingRecord, Relevance, RelevanceCandidate};
use std::sync::Mutex;
use tempfile::TempDir;

/// Scores each candidate by a keyword in its text.
struct KeywordJudge(&'static str);

impl RelevanceJudge for KeywordJudge {
    fn judge<'a>(
        &'a self,
        _query: &'a str,
        candidates: &'a [RelevanceCandidate],
    ) -> PortFuture<'a, Relevance> {
        let scores = candidates
            .iter()
            .map(|c| Some(if c.text.contains(self.0) { 0.95 } else { 0.05 }))
            .collect();
        Box::pin(async move { Relevance::Scored(scores) })
    }
}

#[derive(Default)]
struct RecordingLog(Mutex<Vec<SearchRecord>>);

impl SearchLog for RecordingLog {
    fn record(&self, record: &SearchRecord) {
        self.0.lock().unwrap().push(record.clone());
    }
}

fn workspace() -> TempDir {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("a.rs"), "// retry: see the docs\n").unwrap();
    std::fs::write(
        tmp.path().join("b.rs"),
        "fn retry_delay() -> Duration { BACKOFF * 2 }\n",
    )
    .unwrap();
    tmp
}

fn tool(tmp: &TempDir) -> GrepTool {
    GrepTool::new(
        Arc::new(tmp.path().to_path_buf()),
        Arc::new(Sandbox::new(None)),
    )
}

/// rank_by is offered only where ranking is configured.
#[test]
fn rank_by_is_offered_only_when_ranking_is_configured() {
    let tmp = workspace();
    let plain = tool(&tmp).definition();
    assert!(!plain.parameters_schema.contains("rank_by"));
    assert!(!plain.description.contains("rank_by"));
    let ranked = tool(&tmp)
        .with_relevance(Arc::new(KeywordJudge("BACKOFF")), 30)
        .definition();
    let schema: serde_json::Value = serde_json::from_str(&ranked.parameters_schema).unwrap();
    assert_eq!(schema["properties"]["rank_by"]["type"], "string");
    assert!(
        ranked.description.contains("add rank_by"),
        "{}",
        ranked.description
    );
    assert!(
        ranked
            .description
            .contains("USE THIS TOOL FOR ALL CONTENT SEARCH"),
        "the base description stays"
    );
}

/// With rank_by, the relevant match comes first with its score, and the
/// search is logged with the ranking.
#[tokio::test]
async fn ranked_matches_come_best_first_and_the_search_is_logged() {
    let tmp = workspace();
    let log = Arc::new(RecordingLog::default());
    let tool = tool(&tmp)
        .with_relevance(Arc::new(KeywordJudge("BACKOFF")), 30)
        .with_search_log(log.clone());
    let result = tool
        .execute(r#"{"pattern": "retry", "rank_by": "where the retry delay is computed"}"#)
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    let lines: Vec<&str> = result.content.lines().collect();
    assert_eq!(
        lines[0],
        "[0.95] b.rs:1: fn retry_delay() -> Duration { BACKOFF * 2 }"
    );
    assert_eq!(lines[1], "[0.05] a.rs:1: // retry: see the docs");
    let records = log.0.lock().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].output, "content");
    assert_eq!(records[0].found, 2);
    assert_eq!(records[0].error, None);
    assert_eq!(
        records[0].arguments["rank_by"],
        "where the retry delay is computed"
    );
    match &records[0].ranking {
        Some(RankingRecord::Ranked { query, scores, .. }) => {
            assert_eq!(query, "where the retry delay is computed");
            assert_eq!(scores[0].location, "b.rs:1");
        }
        other => panic!("{other:?}"),
    }
}

/// rank_by where ranking is not configured still searches, in rg order,
/// and says so.
#[tokio::test]
async fn rank_by_without_ranking_configured_still_searches() {
    let tmp = workspace();
    let log = Arc::new(RecordingLog::default());
    let tool = tool(&tmp).with_search_log(log.clone());
    let result = tool
        .execute(r#"{"pattern": "retry", "rank_by": "the delay"}"#)
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert!(result.content.contains("b.rs:1:"), "{}", result.content);
    assert!(
        result.content.contains(grep_rank::NOT_CONFIGURED),
        "{}",
        result.content
    );
    assert!(matches!(
        log.0.lock().unwrap()[0].ranking,
        Some(RankingRecord::NotConfigured { .. })
    ));
}

/// Every call is logged once: a plain search, refused arguments, bad JSON.
#[tokio::test]
async fn every_call_is_logged_once_with_its_outcome() {
    let tmp = workspace();
    let log = Arc::new(RecordingLog::default());
    let tool = tool(&tmp).with_search_log(log.clone());
    tool.execute(r#"{"pattern": "retry", "output": "files"}"#)
        .await
        .unwrap();
    tool.execute(r#"{"pattern": "x", "output": "lines"}"#)
        .await
        .unwrap();
    tool.execute("{not json").await.unwrap();
    let records = log.0.lock().unwrap();
    assert_eq!(records.len(), 3);
    assert_eq!((records[0].output.as_str(), records[0].found), ("files", 2));
    assert_eq!(records[0].ranking, None);
    assert_eq!(records[1].output, "refused");
    assert!(
        records[1]
            .error
            .as_deref()
            .unwrap()
            .contains("output must be one of")
    );
    assert_eq!(records[2].output, "refused");
    assert_eq!(records[2].arguments, serde_json::json!("{not json"));
    assert!(
        records[2]
            .error
            .as_deref()
            .unwrap()
            .contains("invalid JSON")
    );
}
