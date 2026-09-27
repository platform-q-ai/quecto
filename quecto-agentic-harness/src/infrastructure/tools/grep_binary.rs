//! Binary files a directory search skips (#2202). Searching a directory,
//! rg skips each file it finds binary (a NUL byte) and says nothing, so "No
//! matches found" could not tell absent from skipped. Alongside the search a
//! second rg run (the same search with `--binary`, one match per file) finds
//! the binary files that hold a match: rg's `end` record for each says
//! whether it saw binary data. The result then counts them and names the
//! first few. A file named as the path is searched whole by rg, binary or
//! not, so only a directory search is checked.

use std::time::Duration;

use crate::domain::error::DomainError;

use super::grep_run::{AbortOnDrop, CONTENT_STDOUT_CAP, ReadLimit, RgRun, run_rg};

/// How long the check may run on once the search itself is done.
pub(super) const PROBE_GRACE: Duration = Duration::from_secs(2);
/// How many skipped files are named.
const NAMED: usize = 3;
/// How each `end` record in rg's `--json` output starts.
const END_RECORD: &str = r#"{"type":"end""#;

/// What the check found out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Checked {
    /// The search named a file: nothing was skipped.
    NotNeeded,
    /// The binary files holding a match, as rg printed their paths;
    /// `complete` when the check read all of rg's output.
    Found { files: Vec<String>, complete: bool },
    /// The check was abandoned: it outlasted the search by its grace.
    TooSlow,
}

/// A check running alongside the search; ended (and its rg with it) when
/// dropped.
pub(super) struct BinaryProbe {
    task: tokio::task::JoinHandle<Result<RgRun, DomainError>>,
    _abort: AbortOnDrop,
}

impl BinaryProbe {
    /// Start `cmd` (built for the check), bounded like any rg run.
    pub(super) fn start(cmd: tokio::process::Command, timeout: Duration) -> Self {
        let limit = ReadLimit {
            bytes: CONTENT_STDOUT_CAP,
            matches: None,
        };
        let task = tokio::spawn(run_rg(cmd, timeout, limit));
        let abort = AbortOnDrop(task.abort_handle());
        Self {
            task,
            _abort: abort,
        }
    }

    /// What the check found, waiting at most `grace` more for it.
    pub(super) async fn finish(mut self, grace: Duration) -> Checked {
        match tokio::time::timeout(grace, &mut self.task).await {
            Ok(Ok(Ok(run))) => {
                let stdout = String::from_utf8_lossy(&run.stdout);
                // rg finished (2: with errors, which the search reports)
                // and nothing cut its output short.
                let complete = matches!(run.exit_code, Some(0..=2)) && run.cut.is_none();
                Checked::Found {
                    files: binary_files(&stdout),
                    complete,
                }
            }
            // rg could not be run (the search says why), or the task failed.
            Ok(Ok(Err(_)) | Err(_)) => Checked::Found {
                files: Vec::new(),
                complete: false,
            },
            Err(_) => Checked::TooSlow,
        }
    }
}

/// The paths of the files whose `end` record says rg saw binary data in
/// them. A record cut off by the read cap is not read.
pub(super) fn binary_files(stdout: &str) -> Vec<String> {
    use base64::Engine as _;
    stdout
        .lines()
        .filter(|line| line.starts_with(END_RECORD))
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|end| end["data"]["binary_offset"].is_u64())
        .filter_map(|end| {
            let path = &end["data"]["path"];
            match (path["text"].as_str(), path["bytes"].as_str()) {
                (Some(text), _) => Some(text.to_string()),
                (None, Some(encoded)) => base64::engine::general_purpose::STANDARD
                    .decode(encoded)
                    .ok()
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()),
                (None, None) => None,
            }
        })
        .collect()
}

/// The note for the result, when there is something to say: how many
/// skipped binary files hold a match (a floor when the check was cut short)
/// and the first few, as they are shown.
pub(super) fn notice(checked: &Checked) -> Option<String> {
    const UNCHECKED: &str =
        "Binary files are skipped, and they could not all be checked for matches";
    let (files, complete) = match checked {
        Checked::NotNeeded => return None,
        Checked::TooSlow => return Some(UNCHECKED.to_string()),
        Checked::Found { files, complete } => (files, *complete),
    };
    let floor = match complete {
        true => "",
        false => "At least ",
    };
    let named = files
        .iter()
        .take(NAMED)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    let more = match files.len().saturating_sub(NAMED) {
        0 => String::new(),
        more => format!(" (and {more} more)"),
    };
    match (files.len(), complete) {
        (0, true) => None,
        (0, false) => Some(UNCHECKED.to_string()),
        (1, _) => Some(format!(
            "{floor}1 binary file holds a match but was skipped: {named}. Name it as path to search it"
        )),
        (count, _) => Some(format!(
            "{floor}{count} binary files hold matches but were skipped: {named}{more}. Name one as path to search it"
        )),
    }
}
