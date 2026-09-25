//! [`SearchLog`] as local JSONL (#2136 slice B): one line per search in
//! `<base_dir>/search-log/<YYYY-MM-DD>.jsonl` (UTC dates), with the time and
//! session, so search behaviour can always be optimised from logs. Files
//! are private to the user. Recording is best effort: a failure is
//! reported once and never fails a search.

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Value, json};

use crate::application::search::ports::{RankingRecord, SearchLog, SearchRecord};

pub struct JsonlSearchLog {
    dir: PathBuf,
    session: String,
    warned: AtomicBool,
}

impl JsonlSearchLog {
    pub fn new(base_dir: &std::path::Path, session: &str) -> Self {
        Self {
            dir: base_dir.join("search-log"),
            session: session.to_string(),
            warned: AtomicBool::new(false),
        }
    }

    /// Where the log files are.
    pub fn dir(&self) -> &std::path::Path {
        &self.dir
    }

    fn append(&self, record: &SearchRecord) -> std::io::Result<()> {
        let now = humantime::format_rfc3339_seconds(std::time::SystemTime::now()).to_string();
        let mut line = serde_json::to_vec(&line(&now, &self.session, record))?;
        line.push(b'\n');
        create_private_dir(&self.dir)?;
        let date = now.get(..10).unwrap_or("undated");
        let mut options = std::fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(self.dir.join(format!("{date}.jsonl")))?;
        #[cfg(unix)]
        {
            // A file that already existed keeps no looser mode.
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        // One write per record: with O_APPEND, concurrent searches (and
        // sessions) never interleave within a line.
        file.write_all(&line)
    }
}

/// The directory, private to the user.
fn create_private_dir(dir: &std::path::Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

/// One log line: the record's fields, with its time and session. The
/// arguments are embedded as JSON when they are JSON, else as text.
fn line(now: &str, session: &str, record: &SearchRecord) -> Value {
    let arguments = serde_json::from_str::<Value>(&record.arguments)
        .unwrap_or_else(|_| Value::String(record.arguments.clone()));
    json!({
        "ts": now,
        "session": session,
        "tool": "grep",
        "arguments": arguments,
        "output": record.output,
        "found": record.found,
        "incomplete": record.incomplete,
        "error": record.error,
        "elapsed_ms": record.elapsed_ms,
        "ranking": record.ranking.as_ref().map(ranking),
    })
}

fn ranking(record: &RankingRecord) -> Value {
    match record {
        RankingRecord::Ranked {
            query,
            candidates,
            scores,
            unjudged_reason,
            elapsed_ms,
        } => json!({
            "status": "ranked",
            "query": query,
            "candidates": candidates,
            "unjudged_reason": unjudged_reason,
            "scores": scores
                .iter()
                .map(|s| json!({"location": s.location, "score": s.score}))
                .collect::<Vec<_>>(),
            "elapsed_ms": elapsed_ms,
        }),
        RankingRecord::Unavailable {
            query,
            reason,
            elapsed_ms,
        } => json!({
            "status": "unavailable",
            "query": query,
            "reason": reason,
            "elapsed_ms": elapsed_ms,
        }),
        RankingRecord::NotConfigured { query } => {
            json!({"status": "not_configured", "query": query})
        }
    }
}

impl SearchLog for JsonlSearchLog {
    fn record(&self, record: &SearchRecord) {
        if let Err(error) = self.append(record) {
            if !self.warned.swap(true, Ordering::Relaxed) {
                tracing::warn!(dir = %self.dir.display(), %error, "search log could not be written; searches continue unrecorded");
            }
        }
    }
}

#[cfg(test)]
#[path = "search_log_tests.rs"]
mod tests;
