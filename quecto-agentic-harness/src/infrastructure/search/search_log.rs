//! [`SearchLog`] as local JSONL (#2136 slice B): one line per search in
//! `<base_dir>/search-log/<YYYY-MM-DD>.jsonl` (UTC dates), with the time and
//! session, so search behaviour can always be optimised from logs. Files
//! are private to the user. Recording is best effort: a failure is
//! reported once and never fails a search.

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::{Value, json};

use crate::application::search::ports::{SearchLog, SearchRecord};

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

    fn append(&self, record: &SearchRecord) -> std::io::Result<()> {
        let now = humantime::format_rfc3339_seconds(std::time::SystemTime::now()).to_string();
        let mut line = json!({"ts": now, "session": self.session, "tool": "grep"});
        if let (Value::Object(line), Ok(Value::Object(fields))) =
            (&mut line, serde_json::to_value(record))
        {
            line.extend(fields);
        }
        std::fs::create_dir_all(&self.dir)?;
        let date = now.get(..10).unwrap_or("undated");
        let mut options = std::fs::OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(self.dir.join(format!("{date}.jsonl")))?;
        writeln!(file, "{line}")
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
