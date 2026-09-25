//! `rank_by` and the search log through the tool itself (#2136 slice B),
//! over the real rg.
use super::tests::{execute_fake, fake_rg};
use super::*;
use crate::application::search::ports::{
    PortFuture, RankingRecord, Relevance, RelevanceCandidate, SearchRecord,
};
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

impl RecordingLog {
    /// The records, without the retried spawns of a just-written fake rg
    /// (ETXTBSY, see `execute_fake`): each retry is a search of its own.
    fn searches(&self) -> Vec<SearchRecord> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|r| {
                !r.error
                    .as_deref()
                    .is_some_and(|e| e.contains("Text file busy"))
            })
            .cloned()
            .collect()
    }
}

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
    let arguments: serde_json::Value = serde_json::from_str(&records[0].arguments).unwrap();
    assert_eq!(arguments["rank_by"], "where the retry delay is computed");
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
    assert_eq!(records[2].arguments, "{not json");
    assert!(
        records[2]
            .error
            .as_deref()
            .unwrap()
            .contains("invalid JSON")
    );
}

/// Recorded arguments are capped, however long the call.
#[tokio::test]
async fn recorded_arguments_are_capped() {
    let tmp = workspace();
    let log = Arc::new(RecordingLog::default());
    let tool = tool(&tmp).with_search_log(log.clone());
    let long = format!(r#"{{"pattern": "{}"}}"#, "x".repeat(5000));
    tool.execute(&long).await.unwrap();
    assert_eq!(
        log.0.lock().unwrap()[0].arguments.chars().count(),
        crate::application::search::ports::MAX_RECORDED_ARGUMENTS
    );
}

fn with_fake_rg(tmp: &TempDir, script: &str, code: i32, log: Arc<RecordingLog>) -> GrepTool {
    GrepTool::with_rg_binary(
        Arc::new(tmp.path().to_path_buf()),
        Arc::new(Sandbox::new(None)),
        fake_rg(tmp.path(), script, code),
    )
    .with_search_log(log)
}

/// A search dropped before it finishes (an aborted turn) is still recorded,
/// as cancelled.
#[tokio::test]
async fn a_cancelled_search_is_recorded_as_cancelled() {
    let tmp = TempDir::new().unwrap();
    let log = Arc::new(RecordingLog::default());
    let tool = with_fake_rg(&tmp, "exec sleep 5", 0, log.clone());
    let call = execute_fake(&tool, r#"{"pattern": "x"}"#);
    let dropped = tokio::time::timeout(std::time::Duration::from_millis(300), call).await;
    assert!(dropped.is_err(), "the search was still running");
    let records = log.searches();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].output, "cancelled");
    assert!(records[0].incomplete);
    assert_eq!(records[0].arguments, r#"{"pattern": "x"}"#);
}

/// Failures are recorded with their error: rg failing, and rg timing out
/// (a DomainError rather than an error result).
#[tokio::test]
async fn failed_searches_are_recorded_with_their_error() {
    let tmp = TempDir::new().unwrap();
    let log = Arc::new(RecordingLog::default());
    let failing = with_fake_rg(&tmp, "true", 2, log.clone());
    execute_fake(&failing, r#"{"pattern": "x"}"#).await.unwrap();
    let slow = with_fake_rg(&tmp, "exec sleep 5", 0, log.clone())
        .with_rg_timeout(std::time::Duration::from_millis(200));
    execute_fake(&slow, r#"{"pattern": "x"}"#)
        .await
        .unwrap_err();
    let records = log.searches();
    assert_eq!(records.len(), 2);
    assert!(
        records[0]
            .error
            .as_deref()
            .unwrap()
            .contains("Permission denied"),
        "{:?}",
        records[0]
    );
    assert!(
        records[1]
            .error
            .as_deref()
            .unwrap()
            .contains("did not finish"),
        "{:?}",
        records[1]
    );
}

/// A result cut at the read cap is recorded as incomplete.
#[tokio::test]
async fn a_capped_search_is_recorded_as_incomplete() {
    let tmp = TempDir::new().unwrap();
    let log = Arc::new(RecordingLog::default());
    let flood = format!(
        "i=0; while [ $i -lt 20000 ]; do printf '%s/f%05d.rs\\0' '{}' $i; i=$((i+1)); done",
        tmp.path().display()
    );
    let tool = with_fake_rg(&tmp, &flood, 0, log.clone());
    execute_fake(&tool, r#"{"pattern": "x", "output": "files"}"#)
        .await
        .unwrap();
    let records = log.searches();
    assert!(records[0].incomplete, "{:?}", records[0]);
    assert!(records[0].found > 0);
}

/// The search log is never searched, even inside the workspace, while a
/// same-named directory elsewhere still is.
#[tokio::test]
async fn the_search_log_is_never_searched() {
    let tmp = workspace();
    let log_dir = tmp.path().join(".quecto/search-log");
    std::fs::create_dir_all(&log_dir).unwrap();
    std::fs::write(log_dir.join("2026-09-25.jsonl"), r#"{"pattern":"retry"}"#).unwrap();
    std::fs::create_dir_all(tmp.path().join("docs/search-log")).unwrap();
    std::fs::write(tmp.path().join("docs/search-log/notes.jsonl"), "retry").unwrap();
    let tool = tool(&tmp).excluding(log_dir);
    let result = tool
        .execute(r#"{"pattern": "retry", "output": "files"}"#)
        .await
        .unwrap();
    assert!(!result.content.contains(".quecto"), "{}", result.content);
    assert!(
        result.content.contains("docs/search-log/notes.jsonl"),
        "{}",
        result.content
    );
}

/// The log stays out of results however it is reached: through `..` or
/// through a symlink.
#[cfg(unix)]
#[tokio::test]
async fn the_search_log_stays_out_through_dot_dot_and_symlinks() {
    let root = TempDir::new().unwrap();
    let ws = root.path().join("ws");
    let log_dir = root.path().join("base/search-log");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&log_dir).unwrap();
    std::fs::write(log_dir.join("2026-09-25.jsonl"), r#"{"pattern":"retry"}"#).unwrap();
    std::os::unix::fs::symlink(root.path().join("base"), ws.join("link")).unwrap();
    let tool = GrepTool::new(Arc::new(ws.clone()), Arc::new(Sandbox::new(None))).excluding(log_dir);
    for path in ["../base", "link"] {
        let result = tool
            .execute(&format!(
                r#"{{"pattern": "retry", "path": "{path}", "output": "files"}}"#
            ))
            .await
            .unwrap();
        assert_eq!(
            result.content, "No matches found",
            "{path}: {}",
            result.content
        );
    }
}

/// A call dropped before it is ever polled is still recorded, as cancelled.
#[tokio::test]
async fn a_call_dropped_before_it_runs_is_recorded() {
    let tmp = workspace();
    let log = Arc::new(RecordingLog::default());
    let tool = tool(&tmp).with_search_log(log.clone());
    drop(tool.execute(r#"{"pattern": "retry"}"#));
    let records = log.0.lock().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].output, "cancelled");
}

/// A search that panics is recorded as failed, not as cancelled.
#[test]
fn a_panicking_search_is_recorded_as_failed() {
    let log = Arc::new(RecordingLog::default());
    let pending = grep_search::PendingRecord::new(Some(log.clone()), r#"{"pattern":"x"}"#);
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _held = pending;
        panic!("a bug in the search");
    }));
    assert!(outcome.is_err());
    let records = log.0.lock().unwrap();
    assert_eq!(records[0].output, "failed");
    assert_eq!(records[0].error.as_deref(), Some("the search panicked"));
}
