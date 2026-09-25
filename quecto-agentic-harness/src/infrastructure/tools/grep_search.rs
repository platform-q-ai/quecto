//! One grep search (#2136): validate, run rg, read its outcome, rank when
//! asked, format — noting as it goes what the search log records.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::application::search::ports::{
    MAX_RECORDED_ARGUMENTS, RankingRecord, SearchLog, SearchRecord,
};
use crate::domain::error::DomainError;
use crate::domain::tool::ToolResult;
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::path_utils::resolve_to_cwd;
use crate::infrastructure::tools::truncate::format_size;

use super::grep_listing::{ListedFile, ListingFormat, format_listing, parse_listing};
use super::grep_rank::{Ranking, rank};
use super::grep_request::{OutputMode, parse_request};
use super::grep_run::{Cut, RANKED_STDOUT_CAP, RG_STDOUT_CAP, ReadLimit, human_duration, run_rg};
use super::{
    MAX_LINE_BYTES, MAX_OUTPUT_BYTES, MatchFormat, RgMatch, build_rg_command, format_matches,
    parse_rg_matches,
};

/// What rg found, parsed once, outside the excluded directories.
enum Found {
    Matches(Vec<RgMatch>),
    Listing(Vec<ListedFile>),
}

/// What one search needs from its tool.
pub(super) struct SearchContext {
    pub workspace: Arc<PathBuf>,
    pub sandbox: Arc<Sandbox>,
    pub rg_cmd: String,
    pub rg_timeout: std::time::Duration,
    pub ranking: Option<Arc<Ranking>>,
    /// Directories never searched (the search log's own).
    pub excluded: Vec<PathBuf>,
}

/// What the search log records of one search, filled in as it runs.
#[derive(Debug, Default)]
pub(super) struct SearchFacts {
    /// `None` until the arguments are accepted.
    pub output: Option<&'static str>,
    pub found: usize,
    pub incomplete: bool,
    pub ranking: Option<RankingRecord>,
}

fn refused(content: String) -> Result<ToolResult, DomainError> {
    Ok(ToolResult {
        content,
        is_error: true,
        image_blocks: vec![],
        delivery_metadata: None,
    })
}

fn answered(content: String) -> Result<ToolResult, DomainError> {
    Ok(ToolResult {
        content,
        is_error: false,
        image_blocks: vec![],
        delivery_metadata: None,
    })
}

pub(super) async fn search(
    ctx: &SearchContext,
    args: &serde_json::Value,
    facts: &mut SearchFacts,
) -> Result<ToolResult, DomainError> {
    let request = match parse_request(args) {
        Ok(request) => request,
        Err(problem) => return refused(problem),
    };
    facts.output = Some(match request.output {
        OutputMode::Content => "content",
        OutputMode::Files => "files",
        OutputMode::Count => "count",
    });

    let full_path = resolve_to_cwd(&request.path, &ctx.workspace);
    ctx.sandbox
        .validate_path(&full_path.to_string_lossy())
        .map_err(|e| DomainError::Security(e.to_string()))?;

    let cmd = build_rg_command(&ctx.rg_cmd, &ctx.workspace, &full_path, &request);
    // A search that will be ranked (a judge configured, matches asked for)
    // reads as many matches as it may judge (#2142), far past what a plain
    // one shows; the byte cap is only a backstop. `rank_by` with a listing
    // is refused when parsed.
    let limit = match (&request.rank_by, &ctx.ranking, request.output) {
        (Some(_), Some(ranking), OutputMode::Content) => ReadLimit {
            bytes: RANKED_STDOUT_CAP,
            matches: Some(ranking.max_candidates),
        },
        (Some(_), Some(_), OutputMode::Files | OutputMode::Count)
        | (Some(_), None, _)
        | (None, _, _) => ReadLimit {
            bytes: RG_STDOUT_CAP,
            matches: None,
        },
    };
    let rg = run_rg(cmd, ctx.rg_timeout, limit).await?;
    let stderr = String::from_utf8_lossy(&rg.stderr).into_owned();
    // Not copied unless rg printed invalid UTF-8.
    let stdout = String::from_utf8(rg.stdout)
        .unwrap_or_else(|invalid| String::from_utf8_lossy(invalid.as_bytes()).into_owned());
    // What rg found outside the excluded directories (the search log's
    // own), which are never shown or counted. Both sides are resolved, so
    // `..`, a symlink or an aliased home cannot bring the log back.
    let excluded: Vec<PathBuf> = ctx
        .excluded
        .iter()
        .filter_map(|dir| std::fs::canonicalize(dir).ok())
        .collect();
    let mut verdicts: std::collections::HashMap<PathBuf, bool> = Default::default();
    let mut kept = |path: &Path| match excluded.as_slice() {
        [] => true,
        dirs => *verdicts.entry(path.to_path_buf()).or_insert_with(|| {
            let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            searchable(&real, dirs)
        }),
    };
    // Parsed once; the output itself is dropped before any ranking waits.
    let parsed = match request.output {
        OutputMode::Content => {
            let mut matches = parse_rg_matches(&stdout);
            matches.retain(|m| kept(&m.file_path));
            Found::Matches(matches)
        }
        OutputMode::Files | OutputMode::Count => {
            let mut files = parse_listing(&stdout, request.output);
            files.retain(|f| kept(Path::new(&f.path)));
            Found::Listing(files)
        }
    };
    drop(stdout);
    let found = match &parsed {
        Found::Matches(matches) => matches.len(),
        Found::Listing(files) => files.len(),
    };
    facts.found = found;
    // rg exits 0 (matches) or 1 (none). Exit 2 is an error, which may
    // follow partial results (an unreadable file) or nothing but a
    // summary (a mistyped path); no code means killed, here at a read
    // limit or the timeout. Results stand when rg finished, or found
    // something. Cut before one whole match was kept:
    match (rg.cut, found) {
        // its line alone is larger than the cap (a minified file): say so,
        // not an error;
        (Some(Cut::Bytes), 0) => {
            facts.incomplete = true;
            return answered(format!(
                "A matching line is larger than {}, so no match could be shown: \
                 narrow the search with glob or type, or read the file directly",
                format_size(limit.bytes)
            ));
        }
        // every match read was in the search log, which is never searched;
        (Some(Cut::Matches(max)), 0) => {
            facts.incomplete = true;
            return answered(format!(
                "The first {max} matches rg found were all in the search log, which is never \
                 searched: narrow the search with path, glob or type"
            ));
        }
        // too slow to print one: as slow as printing nothing.
        (Some(Cut::Timeout), 0) => {
            return Err(DomainError::Tool(format!(
                "rg did not finish within {}: narrow the search with path, glob or type",
                human_duration(ctx.rg_timeout)
            )));
        }
        (Some(_), 1..) | (None, _) => {}
    }
    let usable = matches!(rg.exit_code, Some(0 | 1)) || found > 0;
    let Some(parsed) = usable.then_some(parsed) else {
        return refused(match (stderr.trim(), rg.exit_code) {
            ("", Some(code)) => format!("rg failed with exit status {code} and no message"),
            ("", None) => "rg exited unexpectedly".to_string(),
            (reported, _) => format!("grep error: {reported}"),
        });
    };
    let mut notices = Vec::new();
    if rg.held_open {
        notices.push(
            "rg's output was still held open after it exited; results may be incomplete"
                .to_string(),
        );
    }
    match (rg.exit_code, rg.cut) {
        // Stopped by a signal other than the tool's own.
        (None, None) => {
            notices.push("rg was stopped by a signal; results are incomplete".to_string());
        }
        (_, Some(Cut::Timeout)) => notices.push(format!(
            "rg did not finish within {}; results are incomplete: narrow with path, glob or type",
            human_duration(ctx.rg_timeout)
        )),
        (Some(_), None) | (_, Some(Cut::Bytes | Cut::Matches(_))) => {}
    }
    if rg.exit_code == Some(2) {
        let first = stderr.lines().next().unwrap_or("").trim();
        notices.push(format!(
            "rg reported errors, results may be incomplete: {first}"
        ));
    }
    facts.incomplete = rg.cut.is_some() || !notices.is_empty();
    // Only a search rg finished normally, whole and with nothing held open,
    // saw every match: a ranking of fewer cannot say none is relevant.
    let complete = matches!(rg.exit_code, Some(0 | 1)) && rg.cut.is_none() && !rg.held_open;

    let result = match parsed {
        Found::Matches(mut matches) => {
            let format = MatchFormat {
                workspace: &ctx.workspace,
                sandbox: &ctx.sandbox,
                match_limit: request.limit,
                context_lines: request.context_lines,
                max_line_bytes: MAX_LINE_BYTES,
                max_output_bytes: MAX_OUTPUT_BYTES,
            };
            if let Some(query) = &request.rank_by {
                let ranked = rank(
                    ctx.ranking.as_deref(),
                    query,
                    matches,
                    &ctx.workspace,
                    &ctx.sandbox,
                    complete,
                )
                .await;
                notices.extend(ranked.notice);
                facts.ranking = Some(ranked.record);
                matches = ranked.matches;
            }
            format_matches(matches, &format).await
        }
        Found::Listing(files) => format_listing(
            files,
            &ListingFormat {
                workspace: &ctx.workspace,
                sandbox: &ctx.sandbox,
                limit: request.limit,
                max_output_bytes: MAX_OUTPUT_BYTES,
                total_is_partial: rg.cut.is_some(),
            },
        ),
    };
    // At a read limit more exists than was read. A ranked search always
    // says so (what was never read was never ranked, so the best match may
    // be among it); otherwise only when the match limit did not already
    // cut the result shorter (its own notice says so). A timeout has said
    // so above.
    let ranked = matches!(facts.ranking, Some(RankingRecord::Ranked { .. }));
    match (rg.cut, ranked, found <= request.limit) {
        (Some(Cut::Matches(max)), true, _) => notices.push(format!(
            "rg found more than {max} matches: the first {max} it found were ranked; narrow with path, glob or type to rank the rest"
        )),
        (Some(Cut::Bytes), true, _) => notices.push(format!(
            "rg printed more than {}: only the matches read before it were ranked; narrow with path, glob or type to rank the rest",
            format_size(limit.bytes)
        )),
        (Some(Cut::Matches(max)), false, true) => notices.push(format!(
            "rg found more than {max} matches; results are incomplete: narrow with path, glob or type"
        )),
        (Some(Cut::Bytes), false, true) => notices.push(format!(
            "rg printed more than {}; results are incomplete: narrow with path, glob or type",
            format_size(limit.bytes)
        )),
        (Some(Cut::Matches(_) | Cut::Bytes), false, false)
        | (Some(Cut::Timeout), _, _)
        | (None, _, _) => {}
    }
    answered(match notices.as_slice() {
        [] => result,
        notices => format!("{result}\n\n[{}]", notices.join(". ")),
    })
}

/// A search's log record, written exactly once: by [`Self::finish`] with
/// what the search did, or, when the call is dropped before it finishes
/// (the turn was aborted), as a cancelled search.
pub(super) struct PendingRecord {
    log: Option<Arc<dyn SearchLog>>,
    arguments: String,
    started: std::time::Instant,
}

impl PendingRecord {
    pub(super) fn new(log: Option<Arc<dyn SearchLog>>, arguments: &str) -> Self {
        Self {
            log,
            arguments: arguments.chars().take(MAX_RECORDED_ARGUMENTS).collect(),
            started: std::time::Instant::now(),
        }
    }

    pub(super) fn finish(mut self, facts: SearchFacts, outcome: &Result<ToolResult, DomainError>) {
        let error = match outcome {
            Ok(result) if result.is_error => Some(result.content.chars().take(500).collect()),
            Ok(_) => None,
            Err(e) => Some(e.to_string()),
        };
        if let Some(log) = self.log.take() {
            log.record(&SearchRecord {
                arguments: std::mem::take(&mut self.arguments),
                output: facts.output.unwrap_or("refused").to_string(),
                found: facts.found,
                incomplete: facts.incomplete,
                error,
                elapsed_ms: elapsed_ms(self.started),
                ranking: facts.ranking,
            });
        }
    }
}

impl Drop for PendingRecord {
    fn drop(&mut self) {
        if let Some(log) = self.log.take() {
            let (output, error) = if std::thread::panicking() {
                ("failed", "the search ended while a panic unwound")
            } else {
                ("cancelled", "the search was cancelled before it finished")
            };
            log.record(&SearchRecord {
                arguments: std::mem::take(&mut self.arguments),
                output: output.to_string(),
                found: 0,
                incomplete: true,
                error: Some(error.to_string()),
                elapsed_ms: elapsed_ms(self.started),
                ranking: None,
            });
        }
    }
}

fn elapsed_ms(started: std::time::Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// A resolved path outside every (resolved) excluded directory.
fn searchable(path: &Path, excluded: &[PathBuf]) -> bool {
    excluded.iter().all(|dir| !path.starts_with(dir))
}
