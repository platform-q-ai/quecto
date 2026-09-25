//! One grep search (#2136): validate, run rg, read its outcome, rank when
//! asked, format — noting as it goes what the search log records.

use std::path::PathBuf;
use std::sync::Arc;

use crate::application::search::ports::RankingRecord;
use crate::domain::error::DomainError;
use crate::domain::tool::ToolResult;
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::path_utils::resolve_to_cwd;
use crate::infrastructure::tools::truncate::format_size;

use super::grep_listing::{ListingFormat, format_listing, parse_listing};
use super::grep_rank::{Ranking, rank};
use super::grep_request::{OutputMode, parse_request};
use super::grep_run::{RG_STDOUT_CAP, run_rg};
use super::{
    MAX_LINE_BYTES, MAX_OUTPUT_BYTES, MatchFormat, build_rg_command, format_matches,
    parse_rg_matches,
};

/// What one search needs from its tool.
pub(super) struct SearchContext {
    pub workspace: Arc<PathBuf>,
    pub sandbox: Arc<Sandbox>,
    pub rg_cmd: String,
    pub rg_timeout: std::time::Duration,
    pub ranking: Option<Arc<Ranking>>,
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
    let rg = run_rg(cmd, ctx.rg_timeout).await?;
    let stderr = String::from_utf8_lossy(&rg.stderr).into_owned();
    let stdout = String::from_utf8_lossy(&rg.stdout).into_owned();
    let found = match request.output {
        OutputMode::Content => parse_rg_matches(&stdout).len(),
        OutputMode::Files | OutputMode::Count => parse_listing(&stdout, request.output).len(),
    };
    facts.found = found;
    // rg exits 0 (matches) or 1 (none). Exit 2 is an error, which may
    // follow partial results (an unreadable file) or nothing but a
    // summary (a mistyped path); no code means killed, here at the
    // output cap. Results stand when rg finished, or found something.
    // Capped before one whole match was read: its line alone is larger
    // than the cap (a minified file) — say so, not an error.
    if rg.capped && found == 0 {
        facts.incomplete = true;
        return answered(format!(
            "A matching line is larger than {}, so no match could be shown: \
             narrow the search with glob or type, or read the file directly",
            format_size(RG_STDOUT_CAP)
        ));
    }
    let usable = matches!(rg.exit_code, Some(0 | 1)) || found > 0;
    let Some(stdout) = usable.then_some(stdout) else {
        return refused(match (stderr.trim(), rg.exit_code) {
            ("", Some(code)) => format!("rg failed with exit status {code} and no message"),
            ("", None) => "rg exited unexpectedly".to_string(),
            (reported, _) => format!("grep error: {reported}"),
        });
    };
    let mut notices = Vec::new();
    // At the cap more exists than was read; say so unless the match
    // limit already cut the result shorter (its own notice says so).
    if rg.capped && found <= request.limit {
        notices.push(format!(
            "rg printed more than {}; results are incomplete: narrow with path, glob or type",
            format_size(RG_STDOUT_CAP)
        ));
    }
    if rg.held_open {
        notices.push(
            "rg's output was still held open after it exited; results may be incomplete"
                .to_string(),
        );
    }
    // Stopped by a signal other than the tool's own at the cap.
    if let (None, false) = (rg.exit_code, rg.capped) {
        notices.push("rg was stopped by a signal; results are incomplete".to_string());
    }
    if rg.exit_code == Some(2) {
        let first = stderr.lines().next().unwrap_or("").trim();
        notices.push(format!(
            "rg reported errors, results may be incomplete: {first}"
        ));
    }
    facts.incomplete = rg.capped || !notices.is_empty();

    let result = match request.output {
        OutputMode::Content => {
            let format = MatchFormat {
                workspace: &ctx.workspace,
                sandbox: &ctx.sandbox,
                match_limit: request.limit,
                context_lines: request.context_lines,
                max_line_bytes: MAX_LINE_BYTES,
                max_output_bytes: MAX_OUTPUT_BYTES,
            };
            let mut matches = parse_rg_matches(&stdout);
            if let Some(query) = &request.rank_by {
                let ranked = rank(ctx.ranking.as_deref(), query, matches, &ctx.workspace).await;
                notices.extend(ranked.notice);
                facts.ranking = Some(ranked.record);
                matches = ranked.matches;
            }
            format_matches(matches, &format).await
        }
        OutputMode::Files | OutputMode::Count => format_listing(
            parse_listing(&stdout, request.output),
            &ListingFormat {
                workspace: &ctx.workspace,
                sandbox: &ctx.sandbox,
                limit: request.limit,
                max_output_bytes: MAX_OUTPUT_BYTES,
                total_is_partial: rg.capped,
            },
        ),
    };
    answered(match notices.as_slice() {
        [] => result,
        notices => format!("{result}\n\n[{}]", notices.join(". ")),
    })
}
