use super::*;
use crate::application::search::ports::{RankingRecord, ScoredLocation};

fn record(found: usize, ranking: Option<RankingRecord>) -> SearchRecord {
    SearchRecord {
        arguments: r#"{"pattern": "retry", "rank_by": "backoff"}"#.into(),
        output: "content".into(),
        found,
        incomplete: false,
        error: None,
        elapsed_ms: 12,
        ranking,
    }
}

/// Every search is one JSON line in the day's file, with its time, session
/// and the record's fields (a ranking with its scores by location).
#[test]
fn each_search_is_one_json_line_in_the_days_file() {
    let base = tempfile::TempDir::new().unwrap();
    let log = JsonlSearchLog::new(base.path(), "chat-1");
    log.record(&record(3, None));
    log.record(&record(
        2,
        Some(RankingRecord::Ranked {
            query: "backoff".into(),
            candidates: 2,
            scores: vec![ScoredLocation {
                location: "a.rs:1".into(),
                score: Some(0.9),
            }],
            unjudged_reason: None,
            elapsed_ms: 40,
        }),
    ));
    let files: Vec<_> = std::fs::read_dir(base.path().join("search-log"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(files.len(), 1, "{files:?}");
    let name = files[0].file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        name.ends_with(".jsonl") && name.len() == "2026-09-25.jsonl".len(),
        "{name}"
    );
    let lines: Vec<Value> = std::fs::read_to_string(&files[0])
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["session"], "chat-1");
    assert_eq!(lines[0]["tool"], "grep");
    assert_eq!(lines[0]["found"], 3);
    assert_eq!(lines[0]["arguments"]["rank_by"], "backoff");
    assert!(lines[0]["ts"].as_str().unwrap().ends_with('Z'));
    assert_eq!(lines[1]["ranking"]["status"], "ranked");
    assert_eq!(lines[1]["ranking"]["scores"][0]["location"], "a.rs:1");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&files[0]).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

/// A log that cannot be written never fails the search.
#[test]
fn an_unwritable_log_is_skipped() {
    let base = tempfile::TempDir::new().unwrap();
    std::fs::write(base.path().join("search-log"), "a file, not a dir").unwrap();
    let log = JsonlSearchLog::new(base.path(), "chat-1");
    log.record(&record(1, None));
    log.record(&record(1, None));
}

/// Arguments that were not JSON are kept as text; a file that already
/// existed with a looser mode is made private.
#[cfg(unix)]
#[test]
fn raw_arguments_are_text_and_an_existing_file_is_made_private() {
    use std::os::unix::fs::PermissionsExt;
    let base = tempfile::TempDir::new().unwrap();
    let log = JsonlSearchLog::new(base.path(), "chat-1");
    log.record(&record(0, None));
    let file = std::fs::read_dir(base.path().join("search-log"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
    let mut raw = record(0, None);
    raw.arguments = "{not json".into();
    log.record(&raw);
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let last: Value = serde_json::from_str(
        std::fs::read_to_string(&file)
            .unwrap()
            .lines()
            .last()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(last["arguments"], "{not json");
}
