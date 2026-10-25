//! Offline fence command; parse the complete allowlist before composing Git.
use crate::{
    application::project_board::{
        ports::{
            BoardHead, BoardStoreError,
            fence::{FenceError, FenceReport},
        },
        use_cases::FenceBoard,
    },
    domain::project_board::value_objects::{board_moved::BoardMoved, telemetry::BoardId},
    interface::cli::CliContext,
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, ffi::OsString, path::PathBuf};

pub const USAGE: &str = "usage: quecto board-server fence --repo <clone> --board-id <16-hex> --url <http(s)> [--url ...] [--remote <name>]";
pub struct FenceCliEnvironment {
    pub repo: PathBuf,
    pub remote: String,
    pub vars: Vec<(OsString, OsString)>,
}
pub struct FenceCliHandles {
    pub fence: FenceBoard,
    pub captured: BoardHead,
}
pub type BoardFenceBuilder = fn(FenceCliEnvironment) -> Result<FenceCliHandles, FenceError>;

fn parse(args: &[String]) -> Result<(FenceCliEnvironment, BoardMoved), String> {
    let mut values = BTreeMap::new();
    let mut urls = Vec::new();
    for pair in args.chunks(2) {
        let [key, value] = pair else {
            return Err("fence: option value required".into());
        };
        if ["--repo", "--remote", "--board-id", "--url"].contains(&key.as_str())
            && value
                .as_bytes()
                .first()
                .is_some_and(|b| (1..=44).contains(b) || (46..=255).contains(b))
            && value.bytes().all(|b| (1..=255).contains(&b))
        {
        } else {
            return Err("fence: unsupported option or missing value".into());
        }
        if key == "--url" {
            urls.push(value.clone());
        } else if values.insert(key.as_str(), value.as_str()).is_none() {
        } else {
            return Err("fence: repeated option".into());
        }
    }
    let repo = PathBuf::from(*values.get("--repo").ok_or("fence: --repo required")?);
    let board = BoardId::of_hex(
        values
            .get("--board-id")
            .ok_or("fence: --board-id required")?,
    )
    .ok_or("fence: invalid board id")?;
    let moved = BoardMoved::new(board, urls)?;
    let remote = values.get("--remote").copied().unwrap_or("origin");
    if (1..=200).contains(&remote.len())
        && remote
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        && remote
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
    {
    } else {
        return Err("fence: invalid remote name".into());
    }
    Ok((
        FenceCliEnvironment {
            repo,
            remote: remote.into(),
            vars: Vec::new(),
        },
        moved,
    ))
}
fn store_error(error: &BoardStoreError) -> Value {
    let kind = match error {
        BoardStoreError::NoBoard => "no_board",
        BoardStoreError::Clash(_) => "clash",
        BoardStoreError::InvalidWrite(_) => "invalid_write",
        BoardStoreError::WriteTooLarge(_) => "write_too_large",
        BoardStoreError::Unavailable(_) => "unavailable",
        BoardStoreError::TooLarge(_) => "too_large",
        BoardStoreError::FolderSetAside { .. } => "folder_set_aside",
        BoardStoreError::MovedMalformed(_) => "moved_malformed",
        BoardStoreError::Moved(found) => {
            let urls: Vec<_> = found.urls().iter().map(|url| safe(url)).collect();
            return json!({"kind":"moved", "board_id":found.board().as_str(), "urls":urls});
        }
    };
    json!({"kind":kind, "message":safe(&format!("{error:?}"))})
}
fn failure(error: &FenceError) -> Value {
    let kind = match error {
        FenceError::Store(error) => return json!({"kind":"store", "store":store_error(error)}),
        FenceError::Conflict => "conflict",
        FenceError::Refused(_) => "refused",
        FenceError::Uncertain(_) => "uncertain",
        FenceError::Unavailable(_) => "unavailable",
    };
    json!({"kind":kind, "message":safe(&format!("{error:?}"))})
}
fn safe(text: &str) -> String {
    use crate::domain::shared::redaction::{redact_secrets, redact_url_userinfo};
    redact_secrets(&redact_url_userinfo(text))
}
pub fn run(ctx: &CliContext, args: &[String], stdout: &mut String, stderr: &mut String) -> i32 {
    if matches!(args, [help] if ["--help", "-h"].contains(&help.as_str())) {
        stdout.push_str(&format!("{USAGE}\n"));
        return 0;
    }
    let (mut environment, moved) = match parse(args) {
        Ok(parsed) => parsed,
        Err(error) => {
            let message = safe(&error);
            stdout.push_str(&format!(
                "{}\n",
                json!({"error":{"kind":"invalid_arguments", "message":message}})
            ));
            stderr.push_str(&format!("{message}\n{USAGE}\n"));
            return 1;
        }
    };
    environment.vars = std::env::vars_os().collect();
    let result = ctx
        .board_fence
        .ok_or_else(|| FenceError::Unavailable("fence: graph is not composed".into()))
        .and_then(|builder| builder(environment))
        .and_then(|handles| handles.fence.execute(&handles.captured, &moved));
    let (code, report) = match result {
        Ok(FenceReport::Already { head }) => (0, json!({"status":"already", "head":head.0})),
        Ok(FenceReport::Published { previous, head }) => (
            0,
            json!({"status":"published", "previous":previous.0, "head":head.0}),
        ),
        Err(error) => {
            stderr.push_str(&format!("fence: {}\n", safe(&format!("{error:?}"))));
            (1, json!({"error":failure(&error)}))
        }
    };
    stdout.push_str(&format!("{report}\n"));
    code
}
#[cfg(test)]
#[path = "fence_tests.rs"]
mod tests;
