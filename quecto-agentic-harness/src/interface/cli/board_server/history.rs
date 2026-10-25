//! Offline native export; dispatched without the listener or token graph.
use crate::application::project_board::use_cases::ExportBoardHistory;
use crate::domain::project_board::value_objects::telemetry::BoardId;
use crate::interface::cli::CliContext;
use std::{collections::BTreeMap, path::PathBuf};

pub struct HistoryCliEnvironment {
    pub repo: PathBuf,
    pub board: BoardId,
    pub target: String,
}
pub struct HistoryCliHandles {
    pub export: ExportBoardHistory,
    pub expected: Option<String>,
}
pub type BoardHistoryBuilder = fn(HistoryCliEnvironment) -> Result<HistoryCliHandles, String>;
pub const USAGE: &str = "usage: quecto board-server export --repo <destination> --board-id <16-hex> --to-branch <branch> --format 3 [--remote <name>] [--dry-run]. Offline native export has no total revision cap. Dry-run writes neither objects nor refs. Publication may leave unreachable verified objects; ambiguous publication requires reconciliation. No distributed DB/Git transaction.";
fn parse(args: &[String]) -> Result<(HistoryCliEnvironment, bool), String> {
    let mut values = BTreeMap::new();
    let mut dry = false;
    let mut i = 0;
    while i < args.len() {
        let key = args[i].as_str();
        if [
            "--repo",
            "--board-id",
            "--to-branch",
            "--format",
            "--remote",
            "--dry-run",
        ]
        .contains(&key)
        {
        } else {
            return Err(format!("unsupported export option {key}"));
        }
        if key == "--dry-run" {
            match dry {
                false => dry = true,
                true => return Err("duplicate --dry-run".into()),
            }
            i += 1;
            continue;
        }
        let value = args
            .get(i + 1)
            .filter(|s| {
                matches!(s.as_bytes(), [_, ..])
                    && s.as_bytes()
                        .first()
                        .is_some_and(|b| (1..=44).contains(b) || (46..=255).contains(b))
            })
            .ok_or_else(|| format!("missing {key}"))?;
        if values.insert(key, value.as_str()).is_none() {
        } else {
            return Err(format!("duplicate {key}"));
        }
        i += 2;
    }
    let repo = PathBuf::from(*values.get("--repo").ok_or("missing --repo")?);
    let board = BoardId::of_hex(values.get("--board-id").ok_or("missing --board-id")?)
        .ok_or("invalid --board-id")?;
    if values.get("--format") == Some(&"3") {
    } else {
        return Err("export requires --format 3".into());
    }
    if let Some(remote) = values.get("--remote") {
        if (1..=200).contains(&remote.len())
            && remote
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            && remote
                .bytes()
                .next()
                .is_some_and(|b| b.is_ascii_alphanumeric())
        {
        } else {
            return Err("invalid --remote".into());
        }
    }
    let branch = values.get("--to-branch").ok_or("missing --to-branch")?;
    if (1..=1000).contains(&branch.len()) {
    } else {
        return Err("invalid --to-branch".into());
    }
    let target = if branch.starts_with("refs/heads/") {
        branch.to_string()
    } else {
        format!("refs/heads/{branch}")
    };
    Ok((
        HistoryCliEnvironment {
            repo,
            board,
            target,
        },
        dry,
    ))
}
pub fn run(ctx: &CliContext, args: &[String], stdout: &mut String, stderr: &mut String) -> i32 {
    if matches!(args, [help] if ["--help","-h"].contains(&help.as_str())) {
        stdout.push_str(USAGE);
        stdout.push('\n');
        return 0;
    }
    let result = (|| -> Result<serde_json::Value, String> {
        let (environment, dry) = parse(args)?;
        let builder = ctx.board_history.ok_or("export history is not composed")?;
        let handles = builder(environment)?;
        let report = handles
            .export
            .execute(dry, handles.expected.as_deref())
            .map_err(|e| e.to_string())?;
        Ok(
            serde_json::json!({"revisions":report.revisions,"frontier":report.frontier.0,"git_head":report.git_head}),
        )
    })();
    match result {
        Ok(report) => {
            stdout.push_str(&format!("{report}\n"));
            0
        }
        Err(why) => {
            stderr.push_str(&format!("board-server: {why}\n{USAGE}\n"));
            1
        }
    }
}

#[cfg(test)]
#[path = "history_tests.rs"]
mod tests;

pub struct ImportCliEnvironment {
    pub repo: PathBuf,
    pub board: BoardId,
    pub remote: String,
}
pub type BoardHistoryImportBuilder =
    fn(
        ImportCliEnvironment,
    ) -> Result<crate::application::project_board::use_cases::ImportBoardHistory, String>;
pub const IMPORT_USAGE: &str = "usage: quecto board-server import --repo <source> --board-id <16-hex> [--remote <name>]. Offline complete local linear history only; remote is provenance, never fetched. Import requires an empty destination. All originals are verified by full dry export before atomic commit.";
fn parse_import(args: &[String]) -> Result<ImportCliEnvironment, String> {
    let mut values = BTreeMap::new();
    let mut pairs = args.chunks_exact(2);
    for pair in &mut pairs {
        let (key, value) = (pair[0].as_str(), pair[1].as_str());
        if ["--repo", "--board-id", "--remote"].contains(&key)
            && value
                .as_bytes()
                .first()
                .is_some_and(|b| (1..=44).contains(b) || (46..=255).contains(b))
            && values.insert(key, value).is_none()
        {
        } else {
            return Err(format!("invalid or duplicate import option {key}"));
        }
    }
    if pairs.remainder().is_empty() {
    } else {
        return Err("missing import option value".into());
    }
    let board = BoardId::of_hex(values.get("--board-id").ok_or("missing --board-id")?)
        .ok_or("invalid --board-id")?;
    let repo = PathBuf::from(*values.get("--repo").ok_or("missing --repo")?);
    let remote = values.get("--remote").copied().unwrap_or("origin");
    if (1..=200).contains(&remote.len())
        && remote
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        && remote
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphanumeric())
    {
    } else {
        return Err("invalid --remote".into());
    }
    Ok(ImportCliEnvironment {
        repo,
        board,
        remote: remote.into(),
    })
}
pub fn run_import(
    ctx: &CliContext,
    args: &[String],
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    if matches!(args, [help] if ["--help","-h"].contains(&help.as_str())) {
        stdout.push_str(IMPORT_USAGE);
        stdout.push('\n');
        return 0;
    }
    let result = (|| -> Result<serde_json::Value, String> {
        let environment = parse_import(args)?;
        let builder = ctx
            .board_history_import
            .ok_or("import history is not composed")?;
        let report = builder(environment)?.execute().map_err(|e| e.to_string())?;
        Ok(
            serde_json::json!({"revisions":report.revisions,"head":report.head.0,"source_head":report.source_head}),
        )
    })();
    match result {
        Ok(report) => {
            stdout.push_str(&format!("{report}\n"));
            0
        }
        Err(why) => {
            stderr.push_str(&format!("board-server: {why}\n{IMPORT_USAGE}\n"));
            1
        }
    }
}
