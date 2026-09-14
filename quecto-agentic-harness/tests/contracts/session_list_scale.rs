//! Deterministic large-directory fixture for List saved sessions (#1861,
//! D1 of #1968): the author's `~/.quecto/sessions` holds ~22k files, so the
//! list must stay one directory walk with the `.json` allowlist and the
//! identity-prefix skip applied before any read, one lightweight header
//! parse per admitted record, and no full load or spill scan per session.
//!
//! The fixture makes that observable rather than assumed: every session
//! record has a header the summary parser accepts but a body the full loader
//! rejects, so a list that took the N+1 full-load path would surface zero
//! sessions (or an error) instead of all of them. Decoy files (`.owner`,
//! `.tmp`, per-session spill directories, a non-JSON note) prove the
//! allowlist runs first.
//!
//! Size: `QUECTO_LIST_SCALE_FILES` (default 400 sessions) — the same-host
//! release comparison of the issue is run with `22000`; with
//! `QUECTO_LIST_SCALE_BENCH=1` the test also reports one warm-up and five
//! measured runs (median and distribution) on stderr for that comparison.

use std::time::{Duration, Instant};

use quecto::application::sessions::dto::SessionListQuery;
use quecto::application::sessions::ports::SessionStore;
use quecto::domain::session_identity::{SessionIdentity, SessionKeyPrefix};
use quecto::infrastructure::persistence::session_layout::FlatSessionLayout;
use quecto::infrastructure::persistence::session_store::FileSessionStore;

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// `count` chat sessions plus `count / 4` legacy `cli:` sessions, every
/// record header-valid but body-malformed, interleaved with decoys.
fn build_fixture(base: &std::path::Path, count: usize) -> (usize, usize) {
    let dir = base.join("sessions");
    std::fs::create_dir_all(&dir).unwrap();
    let mut chats = 0;
    let mut clis = 0;
    for i in 0..count {
        let key = format!("chat-{:010}-{i:x}", 1_700_000_000 + i);
        let record = dir.join(format!("{key}.json"));
        std::fs::write(
            &record,
            format!(
                r#"{{"key":"{key}","messages":[{{"role":"user","content":"question {i}","tool_calls":"not-an-array"}},{{"role":"assistant","content":"answer"}}]}}"#
            ),
        )
        .unwrap();
        // Distinct, deliberately unordered mtimes (written ascending, stamped
        // in a scrambled order) so "newest first" can actually fail.
        let stamp = std::time::UNIX_EPOCH
            + std::time::Duration::from_secs(1_600_000_000 + ((i * 7_919) % count) as u64);
        std::fs::File::options()
            .write(true)
            .open(&record)
            .unwrap()
            .set_modified(stamp)
            .unwrap();
        chats += 1;
        if i % 4 == 0 {
            let name = format!("legacy{i}");
            std::fs::write(
                dir.join(format!("cli_{name}.json")),
                format!(
                    r#"{{"key":"cli:{name}","messages":[{{"role":"user","content":"legacy {i}","tool_calls":"not-an-array"}}]}}"#
                ),
            )
            .unwrap();
            clis += 1;
        }
        if i % 8 == 0 {
            std::fs::write(dir.join(format!("{key}.owner")), b"4242").unwrap();
            std::fs::create_dir_all(dir.join(&key)).unwrap();
            std::fs::write(dir.join(&key).join("spill.jsonl"), b"{}\n").unwrap();
        }
        if i % 16 == 0 {
            std::fs::write(dir.join(format!("{key}.tmp")), b"{").unwrap();
        }
    }
    std::fs::write(dir.join("README.txt"), b"not a session").unwrap();
    std::fs::write(dir.join("corrupt.json"), b"{not json").unwrap();
    (chats, clis)
}

fn median(samples: &mut [Duration]) -> Duration {
    samples.sort();
    samples[samples.len() / 2]
}

#[tokio::test]
async fn large_directory_lists_every_session_from_headers_only() {
    let count = env_usize("QUECTO_LIST_SCALE_FILES", 400);
    let tmp = tempfile::tempdir().unwrap();
    let (chats, clis) = build_fixture(tmp.path(), count);
    let store = FileSessionStore::new(FlatSessionLayout::new(tmp.path()));

    let all = store.list(&SessionListQuery::All).await.unwrap();
    assert_eq!(
        all.len(),
        chats + clis,
        "every header-valid record is listed; a full load would have failed them all"
    );
    assert!(
        all.iter()
            .all(|s| s.message_count >= 1 && !s.title.is_empty()),
        "summaries carry the header-derived title and count"
    );
    assert!(
        all.windows(2)
            .all(|w| w[0].updated_unix_secs >= w[1].updated_unix_secs),
        "newest first"
    );
    let stamps: std::collections::BTreeSet<_> = all.iter().map(|s| s.updated_unix_secs).collect();
    assert!(
        stamps.len() > 1
            && all.first().unwrap().updated_unix_secs > all.last().unwrap().updated_unix_secs,
        "the fixture carries distinct mtimes, so the order assertion is not vacuous"
    );

    let only_chats = store
        .list(&SessionListQuery::ExistingKeyPrefix(
            SessionKeyPrefix::new("chat-").unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(only_chats.len(), chats);
    assert!(only_chats.iter().all(|s| s.key.starts_with("chat-")));

    let only_cli = store
        .list(&SessionListQuery::ExistingKeyPrefix(
            SessionKeyPrefix::new("cli:").unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(only_cli.len(), clis);

    // A listed session is summary-only: the malformed bodies never loaded.
    let first = SessionIdentity::from_persisted_key(all[0].key.as_str());
    assert!(store.load(&first).await.is_err());

    if std::env::var("QUECTO_LIST_SCALE_BENCH").is_ok() {
        let _warmup = store.list(&SessionListQuery::All).await.unwrap();
        let mut samples = Vec::new();
        for _ in 0..5 {
            let started = Instant::now();
            let listed = store.list(&SessionListQuery::All).await.unwrap();
            samples.push(started.elapsed());
            assert_eq!(listed.len(), chats + clis);
        }
        let runs: Vec<String> = samples.iter().map(|d| format!("{d:?}")).collect();
        eprintln!(
            "list_sessions scale bench: files={} sessions={} runs=[{}] median={:?} min={:?} max={:?}",
            count,
            chats + clis,
            runs.join(", "),
            median(&mut samples.clone()),
            samples.iter().min().unwrap(),
            samples.iter().max().unwrap(),
        );
    }
}
