//! Binary files a directory search skips (#2202). Searching a directory,
//! rg skips each file it finds binary (a NUL byte) and says nothing, so "No
//! matches found" could not tell absent from skipped.
//!
//! When a directory search finds nothing, a second rg run lists the files
//! holding a match with `--binary` (the same patterns, flags, globs, types
//! and hidden files; paths only, no line content), bounded by a short time
//! and an output cap (#2251 review). The search found nothing, so each
//! file listed is one it skipped as binary: a text file holding a match
//! would have been found. Two cases break that and are handled or
//! accepted: a file in an excluded directory (the search log's own) is
//! dropped from both runs; a file that changed between the runs may be
//! named wrongly, which a second search of it settles. A path that is not
//! UTF-8 is not named (content output cannot show one either). A search
//! that found matches is never checked: the check would repeat its work.
//!
//! A file named as the path is searched whole by rg, binary or not, so only
//! a directory search is checked. A directory search also stops reading a
//! text file at a NUL byte found after its first matches: its content
//! output's `end` records say where, and the result names such files.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use super::grep_request::OutputMode;
use super::grep_run::{RG_STDOUT_CAP, ReadLimit, run_rg};
use super::grep_search::searched_whole;

/// How long the check may take once the search has found nothing.
pub(super) const CHECK_TIMEOUT: Duration = Duration::from_secs(2);
/// How many files a note names.
const NAMED: usize = 3;
/// How each `end` record in rg's `--json` output starts.
const END_RECORD: &str = r#"{"type":"end""#;

/// What the check found out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Checked {
    /// The search found matches, or named a file: nothing to check.
    NotNeeded,
    /// The binary files holding a match, as rg printed their paths;
    /// `complete` when rg finished cleanly and all it printed was read.
    Found { files: Vec<String>, complete: bool },
}

/// Whether a search is checked for skipped binary files: a directory
/// search that found nothing.
pub(super) fn check_due(searched_a_directory: bool, found: usize) -> bool {
    matches!((searched_a_directory, found), (true, 0))
}

/// Run the check `cmd` (built for it), for at most `timeout`.
pub(super) async fn check(cmd: tokio::process::Command, timeout: Duration) -> Checked {
    let limit = ReadLimit {
        bytes: RG_STDOUT_CAP,
        matches: None,
    };
    match run_rg(cmd, timeout, limit).await {
        // Complete only when rg found every file (0: some, 1: none; 2 is
        // an error, such as an unreadable file) and all of it was read.
        Ok(run) => Checked::Found {
            files: listed_files(&run.stdout),
            complete: searched_whole(run.exit_code, run.cut, run.held_open),
        },
        // rg could not be run: the search itself would have said why.
        Err(_) => Checked::Found {
            files: Vec::new(),
            complete: false,
        },
    }
}

/// The paths in rg's `--null` listing: complete records only (a tail cut
/// off by the read cap is dropped), and only those that are UTF-8.
pub(super) fn listed_files(stdout: &[u8]) -> Vec<String> {
    let mut records: Vec<&[u8]> = stdout.split(|byte| *byte == 0).collect();
    // The piece after the last terminator is incomplete (or empty).
    records.pop();
    records
        .into_iter()
        .filter_map(|record| std::str::from_utf8(record).ok())
        .filter(|path| path.chars().next().is_some())
        .map(str::to_string)
        .collect()
}

/// The note for the result, when there is something to say: how many
/// skipped binary files hold a match (a floor, with examples, when the
/// check was cut short) and the first few, quoted, in `mode`'s words.
pub(super) fn notice(checked: &Checked, mode: OutputMode) -> Option<String> {
    const UNCHECKED: &str =
        "Binary files are skipped, and they could not all be checked for matches";
    let (files, complete) = match checked {
        Checked::NotNeeded => return None,
        Checked::Found { files, complete } => (files, *complete),
    };
    let (one, many) = match mode {
        OutputMode::Content => ("was skipped", "were skipped"),
        OutputMode::Files => ("was not listed", "were not listed"),
        OutputMode::Count => ("was not counted", "were not counted"),
    };
    let advice = |which: &str| match mode {
        OutputMode::Content => format!(". Name {which} as path to see its matches"),
        OutputMode::Count => format!(". Name {which} as path to count its matches"),
        OutputMode::Files => String::new(),
    };
    let named = named(files);
    let (floor, lead, more) = match complete {
        true => ("", ":", more_than_named(files.len())),
        false => ("At least ", ", e.g.", String::new()),
    };
    match (files.len(), complete) {
        (0, true) => None,
        (0, false) => Some(UNCHECKED.to_string()),
        (1, _) => Some(format!(
            "{floor}1 binary file holds a match but {one}{lead} {named}{}",
            advice("it")
        )),
        (count, _) => Some(format!(
            "{floor}{count} binary files hold matches but {many}{lead} {named}{more}{}",
            advice("one")
        )),
    }
}

/// The files a directory search's content output (`stdout`) says rg
/// stopped reading at a NUL byte, among those it `matched`: rg writes an
/// `end` record only for a file it printed a match from. A record cut off
/// by the read cap is not read.
pub(super) fn stopped_files(stdout: &str, matched: &HashSet<PathBuf>) -> Vec<String> {
    stdout
        .lines()
        .filter(|line| line.starts_with(END_RECORD))
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|end| end["data"]["binary_offset"].is_u64())
        .filter_map(|end| end["data"]["path"]["text"].as_str().map(str::to_string))
        .filter(|path| matched.contains(&PathBuf::from(path)))
        .collect()
}

/// The note naming files rg stopped reading at a NUL byte, when any.
pub(super) fn stopped_notice(files: &[String]) -> Option<String> {
    match files.len() {
        0 => None,
        1 => Some(format!(
            "rg stopped reading 1 file at a NUL byte, after its first matches: {}. Name it as \
             path to search it whole",
            named(files)
        )),
        count => Some(format!(
            "rg stopped reading {count} files at a NUL byte, after their first matches: {}{}. \
             Name one as path to search it whole",
            named(files),
            more_than_named(count)
        )),
    }
}

/// The first few of `files`, each quoted and escaped (a `]`, a quote or a
/// newline in a name cannot end the note or break its line).
fn named(files: &[String]) -> String {
    files
        .iter()
        .take(NAMED)
        .map(|file| format!("{file:?}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// How many of `count` files a note leaves unnamed, when any.
fn more_than_named(count: usize) -> String {
    match count.saturating_sub(NAMED) {
        0 => String::new(),
        more => format!(" (and {more} more)"),
    }
}
