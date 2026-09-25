//! How much of rg's output a search reads (#2142): a ranked search reads as
//! many matches as it may judge, with a byte backstop; a plain one reads up
//! to the plain-results cap; a timeout keeps what was printed.

use super::rank_by_tests::{KeywordJudge, RecordingLog, with_fake_rg};
use super::tests::execute_fake;
use super::*;
use crate::application::search::ports::{PortFuture, RankingRecord, Relevance, RelevanceCandidate};
use tempfile::TempDir;

/// A judge that is down.
struct DownJudge;

impl RelevanceJudge for DownJudge {
    fn judge<'a>(
        &'a self,
        _query: &'a str,
        _candidates: &'a [RelevanceCandidate],
    ) -> PortFuture<'a, Relevance> {
        Box::pin(async { Relevance::Unavailable("TypeSafe is down".into()) })
    }
}

/// One rg JSON match record for `<dir>/a.rs:1` whose line is `text`.
fn match_record(dir: &std::path::Path, text: &str) -> String {
    format!(
        r#"{{"type":"match","data":{{"path":{{"text":"{}/a.rs"}},"lines":{{"text":"{text}\\n"}},"line_number":1,"absolute_offset":0,"submatches":[{{"match":{{"text":"retry"}},"start":0,"end":5}}]}}}}"#,
        dir.display()
    )
}

/// A fake rg script printing `lines` JSON match records for `a.rs:1`
/// (about 230 bytes each).
fn match_flood(tmp: &TempDir, lines: usize) -> String {
    let record = match_record(tmp.path(), "retry");
    format!("yes '{record}' | head -n {lines}")
}

/// #2142: a ranked search reads rg's output well past the plain-results
/// cap (200 KB), so every match is judged, not just those rg printed
/// first.
#[tokio::test]
async fn a_ranked_search_reads_past_the_plain_results_cap() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("a.rs"), "retry\n").unwrap();
    // About 700 KB of rg output.
    let flood = match_flood(&tmp, 3000);
    let log = Arc::new(RecordingLog::default());
    let tool = with_fake_rg(&tmp, &flood, 0, log.clone())
        .with_relevance(Arc::new(KeywordJudge("retry")), 5000);
    let result = execute_fake(&tool, r#"{"pattern": "retry", "rank_by": "the retry"}"#)
        .await
        .unwrap();
    assert!(
        !result.content.contains("rg printed more than"),
        "{}",
        result.content
    );
    let records = log.searches();
    assert_eq!(records[0].found, 3000);
    assert!(!records[0].incomplete);
    match &records[0].ranking {
        Some(RankingRecord::Ranked { candidates, .. }) => assert_eq!(*candidates, 3000),
        other => panic!("{other:?}"),
    }
}

/// With a judge configured, a plain search (no `rank_by`) still reads only
/// up to the plain-results cap.
#[tokio::test]
async fn a_plain_search_keeps_the_plain_results_cap() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("a.rs"), "retry\n").unwrap();
    let flood = match_flood(&tmp, 3000);
    let tool = with_fake_rg(&tmp, &flood, 0, Arc::new(RecordingLog::default()))
        .with_relevance(Arc::new(KeywordJudge("retry")), 5000);
    let result = execute_fake(&tool, r#"{"pattern": "retry", "limit": 100000}"#)
        .await
        .unwrap();
    let tail = &result.content[result.content.len().saturating_sub(600)..];
    assert!(tail.contains("rg printed more than 200.0KB"), "{tail}");
    assert!(tail.contains("results are incomplete"), "{tail}");
}

/// Without a judge nothing was ranked: a search cut at the read cap says its
/// results are incomplete, never that the matches read were ranked.
#[tokio::test]
async fn an_unranked_search_cut_at_the_read_cap_claims_no_ranking() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("a.rs"), "retry\n").unwrap();
    let flood = match_flood(&tmp, 3000);
    let tool = with_fake_rg(&tmp, &flood, 0, Arc::new(RecordingLog::default()));
    let result = execute_fake(
        &tool,
        r#"{"pattern": "retry", "rank_by": "the retry", "limit": 100000}"#,
    )
    .await
    .unwrap();
    let tail = &result.content[result.content.len().saturating_sub(600)..];
    assert!(!tail.contains("were ranked"), "{tail}");
    assert!(tail.contains("results are incomplete"), "{tail}");
}

/// A ranked search stops reading at as many matches as it may judge, and
/// says the first that many were ranked; poor scores then make no claim
/// that none is relevant, since the rest were never read.
#[tokio::test]
async fn a_ranked_search_stops_at_the_matches_it_may_judge() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("a.rs"), "retry\n").unwrap();
    let flood = match_flood(&tmp, 1200);
    let log = Arc::new(RecordingLog::default());
    let tool = with_fake_rg(&tmp, &flood, 0, log.clone())
        .with_relevance(Arc::new(KeywordJudge("nowhere")), 1000);
    let result = execute_fake(&tool, r#"{"pattern": "retry", "rank_by": "the retry"}"#)
        .await
        .unwrap();
    let tail = &result.content[result.content.len().saturating_sub(600)..];
    assert!(
        tail.contains(
            "rg found more than 1000 matches: only the first 1000 were read, so only those were ranked"
        ),
        "{tail}"
    );
    assert!(!tail.contains("looks relevant"), "{tail}");
    assert!(!tail.contains("ranked the first"), "{tail}");
    let records = log.searches();
    assert_eq!(records[0].found, 1000);
    assert!(records[0].incomplete);
    match &records[0].ranking {
        Some(RankingRecord::Ranked { candidates, .. }) => assert_eq!(*candidates, 1000),
        other => panic!("{other:?}"),
    }
}

/// Exactly as many matches as may be judged: the search is whole.
#[tokio::test]
async fn exactly_the_matches_it_may_judge_is_a_whole_search() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("a.rs"), "retry\n").unwrap();
    let flood = match_flood(&tmp, 1000);
    let tool = with_fake_rg(&tmp, &flood, 0, Arc::new(RecordingLog::default()))
        .with_relevance(Arc::new(KeywordJudge("nowhere")), 1000);
    let result = execute_fake(&tool, r#"{"pattern": "retry", "rank_by": "the retry"}"#)
        .await
        .unwrap();
    let tail = &result.content[result.content.len().saturating_sub(600)..];
    assert!(!tail.contains("rg found more than"), "{tail}");
    assert!(tail.contains("no judged match looks relevant"), "{tail}");
}

/// Very long lines reach the byte backstop before the match cap: the notice
/// names the 16 MiB read.
#[tokio::test]
async fn a_ranked_search_of_long_lines_stops_at_the_byte_backstop() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("a.rs"), "retry\n").unwrap();
    let long = format!("retry{}", "x".repeat(1024 * 1024));
    std::fs::write(
        tmp.path().join("long.json"),
        match_record(tmp.path(), &long) + "\n",
    )
    .unwrap();
    let flood = format!(
        "i=0; while [ $i -lt 20 ]; do cat '{}/long.json'; i=$((i+1)); done",
        tmp.path().display()
    );
    let tool = with_fake_rg(&tmp, &flood, 0, Arc::new(RecordingLog::default()))
        .with_relevance(Arc::new(KeywordJudge("retry")), 1000);
    let result = execute_fake(&tool, r#"{"pattern": "retry", "rank_by": "the retry"}"#)
        .await
        .unwrap();
    let tail = &result.content[result.content.len().saturating_sub(600)..];
    assert!(
        tail.contains("rg printed more than 16.0MB: only the matches read before it were ranked"),
        "{tail}"
    );
}

/// With the judge down nothing was ranked: a search cut at the match cap
/// says its results are incomplete, not that matches were ranked.
#[tokio::test]
async fn a_match_cut_search_the_judge_could_not_rank_says_it_is_incomplete() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("a.rs"), "retry\n").unwrap();
    let flood = match_flood(&tmp, 30);
    let tool = with_fake_rg(&tmp, &flood, 0, Arc::new(RecordingLog::default()))
        .with_relevance(Arc::new(DownJudge), 20);
    let result = execute_fake(
        &tool,
        r#"{"pattern": "retry", "rank_by": "the retry", "limit": 100}"#,
    )
    .await
    .unwrap();
    let tail = &result.content[result.content.len().saturating_sub(600)..];
    assert!(
        tail.contains("rg found more than 20 matches; results are incomplete"),
        "{tail}"
    );
    assert!(!tail.contains("were ranked"), "{tail}");
}

/// When every match read before the cap was in the search log, the search
/// says so rather than reporting rg as failed.
#[tokio::test]
async fn a_match_cap_filled_by_the_search_log_says_so() {
    let tmp = TempDir::new().unwrap();
    let log_dir = tmp.path().join("search-log");
    std::fs::create_dir(&log_dir).unwrap();
    std::fs::write(log_dir.join("a.rs"), "retry\n").unwrap();
    let flood = format!("yes '{}' | head -n 30", match_record(&log_dir, "retry"));
    let tool = with_fake_rg(&tmp, &flood, 0, Arc::new(RecordingLog::default()))
        .with_relevance(Arc::new(KeywordJudge("retry")), 20)
        .excluding(log_dir);
    let result = execute_fake(&tool, r#"{"pattern": "retry", "rank_by": "the retry"}"#)
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert!(
        result
            .content
            .contains("None of the first 20 matches rg found can be shown"),
        "{}",
        result.content
    );
}

/// rg past its timeout: what it printed stands, with a notice; with nothing
/// printed the search still fails as too slow.
#[tokio::test]
async fn a_timed_out_search_keeps_what_rg_printed() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("a.rs"), "retry\n").unwrap();
    let slow = format!(
        "printf '%s\\n' '{}'; exec sleep 30",
        match_record(tmp.path(), "retry")
    );
    let tool = with_fake_rg(&tmp, &slow, 0, Arc::new(RecordingLog::default()))
        .with_rg_timeout(std::time::Duration::from_millis(500));
    let result = execute_fake(&tool, r#"{"pattern": "retry"}"#)
        .await
        .unwrap();
    assert!(result.content.contains("a.rs:1"), "{}", result.content);
    assert!(
        result
            .content
            .contains("rg did not finish within 500 ms; results are incomplete"),
        "{}",
        result.content
    );
    // Something printed but no whole match: as slow as printing nothing.
    let partial = r#"printf '%s' '{"type":"begin"'; exec sleep 30"#;
    let tool = with_fake_rg(&tmp, partial, 0, Arc::new(RecordingLog::default()))
        .with_rg_timeout(std::time::Duration::from_millis(500));
    let error = execute_fake(&tool, r#"{"pattern": "retry"}"#)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("rg did not finish within 500 ms"), "{error}");
}

/// rg may print everything and exit 0 before the match cap is noticed (the
/// output fits the pipe): the search is still cut, so it makes no claim
/// that none is relevant.
#[tokio::test]
async fn a_match_cut_after_rg_exited_is_still_incomplete() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("a.rs"), "retry\n").unwrap();
    let flood = match_flood(&tmp, 30);
    let tool = with_fake_rg(&tmp, &flood, 0, Arc::new(RecordingLog::default()))
        .with_relevance(Arc::new(KeywordJudge("nowhere")), 20);
    let result = execute_fake(&tool, r#"{"pattern": "retry", "rank_by": "the retry"}"#)
        .await
        .unwrap();
    let tail = &result.content[result.content.len().saturating_sub(600)..];
    assert!(tail.contains("rg found more than 20 matches"), "{tail}");
    assert!(!tail.contains("looks relevant"), "{tail}");
}

/// Only a normal exit with nothing cut and nothing held open is a whole
/// search; a cut counts even when rg exited 0 before it was acted on.
#[test]
fn a_search_is_whole_only_when_rg_finished_and_nothing_cut_it() {
    use super::grep_run::Cut;
    use super::grep_search::searched_whole;
    assert!(searched_whole(Some(0), None, false));
    assert!(searched_whole(Some(1), None, false));
    for cut in [Cut::Bytes, Cut::Matches(20), Cut::Timeout] {
        assert!(!searched_whole(Some(0), Some(cut), false), "{cut:?}");
        assert!(!searched_whole(None, Some(cut), false), "{cut:?}");
    }
    assert!(!searched_whole(Some(2), None, false));
    assert!(!searched_whole(None, None, false));
    assert!(!searched_whole(Some(0), None, true));
}

/// A long line arriving over many reads is scanned once, not again from its
/// start on every read; a match split across reads is counted once, when
/// its line ends.
#[test]
fn the_match_counter_scans_each_byte_once() {
    use super::grep_run::MatchLines;
    let record = br#"{"type":"match","data":{}}"#;
    let mut lines = MatchLines::default();
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&record[..10]);
    assert_eq!(lines.find_past(&bytes, 1), None);
    assert_eq!(lines.scanned(), bytes.len());
    bytes.extend_from_slice(&record[10..]);
    bytes.push(b'\n');
    assert_eq!(lines.find_past(&bytes, 1), None);
    assert_eq!(lines.scanned(), bytes.len());
    let second = bytes.len();
    bytes.extend_from_slice(br#"{"type":"context"}"#);
    bytes.push(b'\n');
    bytes.extend_from_slice(record);
    assert_eq!(lines.find_past(&bytes, 1), None, "a line not yet ended");
    assert_eq!(lines.scanned(), bytes.len());
    bytes.push(b'\n');
    let past = second + br#"{"type":"context"}"#.len() + 1;
    assert_eq!(lines.find_past(&bytes, 1), Some(past));
}

/// A ranked search past its rg timeout says only what rg printed was
/// ranked.
#[tokio::test]
async fn a_ranked_search_past_the_timeout_says_only_what_was_printed_was_ranked() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("a.rs"), "retry\n").unwrap();
    let slow = format!(
        "printf '%s\\n' '{}'; exec sleep 30",
        match_record(tmp.path(), "retry")
    );
    let tool = with_fake_rg(&tmp, &slow, 0, Arc::new(RecordingLog::default()))
        .with_relevance(Arc::new(KeywordJudge("retry")), 1000)
        .with_rg_timeout(std::time::Duration::from_millis(500));
    let result = execute_fake(&tool, r#"{"pattern": "retry", "rank_by": "the retry"}"#)
        .await
        .unwrap();
    assert!(
        result
            .content
            .contains("rg did not finish within 500 ms: only the matches it printed were ranked"),
        "{}",
        result.content
    );
}

/// A cut already made stands when only ending rg then outlasts the timeout
/// (here a background child keeps stderr open): the search says what cut
/// it, not that rg never finished.
#[tokio::test]
async fn a_cut_made_before_the_timeout_is_reported_as_the_cut() {
    let tmp = TempDir::new().unwrap();
    std::fs::write(tmp.path().join("a.rs"), "retry\n").unwrap();
    let flood = format!("sleep 30 & {}", match_flood(&tmp, 30));
    let tool = with_fake_rg(&tmp, &flood, 0, Arc::new(RecordingLog::default()))
        .with_relevance(Arc::new(KeywordJudge("retry")), 20)
        .with_rg_timeout(std::time::Duration::from_millis(500));
    let result = execute_fake(&tool, r#"{"pattern": "retry", "rank_by": "the retry"}"#)
        .await
        .unwrap();
    let tail = &result.content[result.content.len().saturating_sub(600)..];
    assert!(tail.contains("rg found more than 20 matches"), "{tail}");
    assert!(!tail.contains("did not finish"), "{tail}");
}

/// An 8 MiB line arriving 8 KiB at a time costs one pass: rescanning it
/// from its start on every read would take seconds (#2142 review).
#[test]
fn a_long_line_arriving_in_pieces_is_counted_in_one_pass() {
    use super::grep_run::MatchLines;
    let mut lines = MatchLines::default();
    let mut bytes = Vec::with_capacity(8 * 1024 * 1024);
    let piece = [b'x'; 8192];
    let started = std::time::Instant::now();
    for _ in 0..1024 {
        bytes.extend_from_slice(&piece);
        assert_eq!(lines.find_past(&bytes, 1), None);
    }
    assert!(
        started.elapsed() < std::time::Duration::from_secs(1),
        "{:?}",
        started.elapsed()
    );
}
