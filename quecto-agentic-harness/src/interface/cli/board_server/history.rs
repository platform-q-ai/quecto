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
mod tests {
    use super::*;
    fn args() -> Vec<String> {
        [
            "--repo",
            "/scratch/destination",
            "--board-id",
            "0123456789abcdef",
            "--to-branch",
            "rollback",
            "--format",
            "3",
            "--dry-run",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    }
    #[test]
    fn export_parser_is_allowlisted_and_help_needs_no_builder() {
        let (environment, dry) = parse(&args()).unwrap();
        assert_eq!(environment.repo, PathBuf::from("/scratch/destination"));
        assert_eq!(environment.board.as_str(), "0123456789abcdef");
        assert_eq!(environment.target, "refs/heads/rollback");
        assert!(dry);
        for extra in [
            vec!["--dry-run"],
            vec!["--repo"],
            vec!["--remote", "-unsafe"],
            vec!["--socket", "/scratch/socket"],
        ] {
            let mut command = args();
            command.extend(extra.into_iter().map(str::to_owned));
            assert!(parse(&command).is_err());
        }
        let (mut out, mut err) = (String::new(), String::new());
        assert_eq!(
            run(
                &CliContext::default(),
                &["--help".into()],
                &mut out,
                &mut err
            ),
            0
        );
        assert!(out.contains("board-server export"));
        assert!(err.is_empty());
    }
    #[test]
    fn export_dispatch_uses_only_the_injected_export_builder() {
        fn build(e: HistoryCliEnvironment) -> Result<HistoryCliHandles, String> {
            assert_eq!(e.board.as_str(), "0123456789abcdef");
            assert_eq!(e.target, "refs/heads/rollback");
            Err("injected offline export graph".into())
        }
        let ctx = CliContext {
            board_history: Some(build),
            ..Default::default()
        };
        let mut command = vec!["export".into()];
        command.extend(args());
        let (mut out, mut err) = (String::new(), String::new());
        assert_eq!(
            super::super::cmd_board_server(&ctx, &command, &mut out, &mut err),
            1
        );
        assert!(err.contains("injected offline export graph"));
        assert!(out.is_empty());
    }
}
