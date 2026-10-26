//! Allowlisted offline backup command; parse completely before composition or I/O.
use crate::{
    application::project_board::{ports::backup::*, use_cases::BackupBoard},
    domain::project_board::value_objects::{telemetry::BoardId, timestamp::Timestamp},
    interface::cli::CliContext,
};
use std::path::PathBuf;
pub struct BackupCliEnvironment {
    pub base_dir: PathBuf,
    pub state: Option<PathBuf>,
    pub board: Option<String>,
}
pub type BoardBackupBuilder = fn(BackupCliEnvironment) -> Result<BackupBoard, String>;
pub const USAGE: &str =
    "board-server backup --to <dir> [--board-id <16-hex>] [--state <absolute-dir>]";
struct Options {
    to: PathBuf,
    board: Option<String>,
    state: Option<PathBuf>,
}
fn parse(args: &[String]) -> Result<Options, &'static str> {
    let mut to = None;
    let mut board = None;
    let mut state = None;
    let mut args = args.iter();
    while let Some(option) = args.next() {
        let value = args
            .next()
            .filter(|s| {
                s.chars().next().is_some_and(|c| {
                    c.is_alphanumeric() || matches!(c, '/' | '.' | '_' | '~' | ' ')
                })
            })
            .ok_or("backup: option value required")?;
        match (
            option.as_str(),
            to.is_none(),
            board.is_none(),
            state.is_none(),
        ) {
            // Named absolutely: SQLite reads a relative `file:..` as a URI.
            ("--to", true, _, _) => to = std::path::absolute(value).ok(),
            ("--board-id", _, true, _) => {
                BoardId::of_hex(value).ok_or("backup: invalid board id")?;
                board = Some(value.clone());
            }
            ("--state", _, _, true) => {
                let path = PathBuf::from(value);
                if path.is_absolute() {
                    state = Some(path);
                } else {
                    return Err("backup: --state must be a non-empty absolute path");
                }
            }
            _ => return Err("backup: unsupported or repeated option"),
        }
    }
    if state.is_none() || board.is_some() {
    } else {
        return Err("backup: --state requires --board-id");
    }
    Ok(Options {
        to: to.ok_or("backup: --to is required")?,
        board,
        state,
    })
}
pub fn run(ctx: &CliContext, args: &[String], stdout: &mut String, stderr: &mut String) -> i32 {
    let options = match parse(args) {
        Ok(options) => options,
        Err(error) => {
            stderr.push_str(&format!("{error}\n{USAGE}\n"));
            return 1;
        }
    };
    let result = ctx
        .board_backup
        .ok_or_else(|| "backup: offline graph not composed".to_owned())
        .and_then(|build| {
            build(BackupCliEnvironment {
                base_dir: ctx.base_dir(),
                state: options.state,
                board: options.board.clone(),
            })
        });
    let usecase = match result {
        Ok(usecase) => usecase,
        Err(_) => {
            stderr.push_str("backup: offline graph unavailable\n");
            return 1;
        }
    };
    let request = BackupRequest {
        board: options.board,
        to: options.to,
        at: Timestamp::parse("1970-01-01T00:00:00Z").expect("fixed epoch"),
        limits: BackupLimits::default(),
    };
    match usecase.execute(&request) {
        Ok(report) => {
            let success = report.outcomes.iter().all(|o| o.error.is_none());
            let outcomes: Vec<_> = report.outcomes.iter().map(|o| serde_json::json!({"board": o.board, "published": o.published, "taken_at": o.taken_at.as_str(), "retained": o.retained, "deleted": o.deleted, "stage": o.stage, "error": o.error})).collect();
            stdout.push_str(&format!("{}\n", serde_json::json!({"outcomes":outcomes})));
            if success { 0 } else { 1 }
        }
        Err(error) => {
            stdout.push_str(&format!("{}\n", serde_json::json!({"error": error})));
            1
        }
    }
}
#[cfg(test)]
#[path = "backup_tests.rs"]
mod tests;
