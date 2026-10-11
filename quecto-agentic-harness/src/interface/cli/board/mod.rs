//! `quecto board <op> [--<field> <value>]... [--input <json>|-]` (#2477):
//! the board operations for clients that are not agents (the pocket
//! relay, scripts, bootstrapping a board). A thin front end over the
//! shared [`project_board`](crate::interface::tools::project_board) path:
//! the request is built from the command line (or taken whole from
//! `--input`), read before anything else is built, run, and its JSON
//! answer printed on one line. It exits 0 when the answer is
//! `"ok": true`, 1 otherwise.
//!
//! `--repo <path>` names the repository (default: the working directory)
//! and `--remote <name>` its board remote (default: `origin`). `--json`
//! is accepted and changes nothing: the answer is always JSON. A read
//! needs no committer; a write needs the repository's configured one.
//!
//! **Trust.** The board is cooperative: a change is attributed to the
//! local git identity, which anyone can set, and anyone who can push the
//! board branch could push raw commits. Branch protection or a server hook
//! is the real control.
use std::io::Read;
use std::path::PathBuf;

use serde_json::Value;

use super::CliContext;
use crate::application::project_board::operations::BoardOperations;
use crate::interface::tools::project_board::answer::{ErrorCode, reason_refused, request_refused};
use crate::interface::tools::project_board::request::{RequestError, op_of, parse};
use crate::interface::tools::project_board::{command_line, json, run};

/// The most bytes `--input -` reads.
pub const MAX_REQUEST_BYTES: u64 = 1024 * 1024;

/// What a run of `quecto board` does with the board.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Reads only: no committer is needed.
    Read,
    /// Writes: the repository's configured committer writes.
    Write,
}

/// What composition is given to build the board's operations.
pub struct ProjectBoardEnvironment {
    pub repo: PathBuf,
    pub remote: String,
    pub access: Access,
    /// The process environment, which composition allowlists for git.
    pub vars: Vec<(std::ffi::OsString, std::ffi::OsString)>,
}

/// The board's operations, built by composition.
pub struct ProjectBoardHandles {
    pub operations: BoardOperations,
}

/// Why composition could not build the board's operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildError {
    /// A write, with no committer configured.
    NoCommitter(String),
    Unavailable(String),
}

pub type ProjectBoardHandlesBuilder =
    fn(&ProjectBoardEnvironment) -> Result<ProjectBoardHandles, BuildError>;

const USAGE: &str = "usage: quecto board <op> [--<field> <value>]... [--input <json>|-] \
                     [--repo <path>] [--remote <name>] [--json]\n";

/// The command line, read.
struct Line {
    op: Option<String>,
    fields: Vec<String>,
    input: Option<String>,
    repo: Option<PathBuf>,
    remote: String,
}

fn read_line(args: &[String]) -> Result<Line, RequestError> {
    let mut line = Line {
        op: None,
        fields: Vec::new(),
        input: None,
        repo: None,
        remote: "origin".into(),
    };
    let mut rest = args.iter();
    let missing = |flag: &str| RequestError {
        field: flag.trim_start_matches('-').into(),
        problem: "needs a value".into(),
    };
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--json" => {}
            "--input" => line.input = Some(rest.next().ok_or_else(|| missing("--input"))?.clone()),
            "--repo" => {
                line.repo = Some(PathBuf::from(rest.next().ok_or_else(|| missing("--repo"))?))
            }
            "--remote" => line.remote = rest.next().ok_or_else(|| missing("--remote"))?.clone(),
            flag if flag.starts_with("--") => {
                let value = rest.next().ok_or_else(|| missing(flag))?;
                line.fields.extend([flag.to_string(), value.clone()]);
            }
            op if line.op.is_none() => line.op = Some(op.to_string()),
            other => {
                return Err(RequestError {
                    field: other.into(),
                    problem: "is not a flag; fields are --<field> <value>".into(),
                });
            }
        }
    }
    Ok(line)
}

/// The request the command line names; `stdin` reads `--input -`.
fn request(
    line: &Line,
    stdin: impl FnOnce() -> std::io::Result<String>,
) -> Result<Value, RequestError> {
    let invalid = |field: &str, problem: &str| RequestError {
        field: field.into(),
        problem: problem.into(),
    };
    match (&line.input, &line.op) {
        (Some(_), _) if !line.fields.is_empty() => Err(invalid(
            "input",
            "is the whole request: give fields there, not as flags",
        )),
        (Some(input), op) => {
            let text = match input.as_str() {
                "-" => stdin()
                    .map_err(|error| invalid("input", &format!("could not be read: {error}")))?,
                text => text.to_string(),
            };
            let mut value = json::strict(&text)
                .map_err(|error| invalid("input", &format!("must be JSON: {error}")))?;
            let object = value
                .as_object_mut()
                .ok_or_else(|| invalid("input", "must be a JSON object"))?;
            if let Some(op) = op {
                let op = Value::String(op.replace('-', "_"));
                match object.get("op") {
                    None => {
                        object.insert("op".into(), op);
                    }
                    Some(named) if *named == op => {}
                    Some(_) => return Err(invalid("op", "differs from the input's op")),
                }
            }
            Ok(value)
        }
        (None, Some(op)) => command_line::request(op, &line.fields),
        (None, None) => Err(invalid("op", "is required")),
    }
}

/// At most [`MAX_REQUEST_BYTES`] of `input`, as text.
fn bounded(input: impl Read) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    input.take(MAX_REQUEST_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_REQUEST_BYTES {
        return Err(std::io::Error::other(format!(
            "it is more than {MAX_REQUEST_BYTES} bytes"
        )));
    }
    String::from_utf8(bytes).map_err(|_| std::io::Error::other("it is not UTF-8"))
}

pub(crate) fn cmd_board(
    ctx: &CliContext,
    args: &[String],
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    if matches!(
        args.first().map(String::as_str),
        None | Some("help" | "--help" | "-h")
    ) {
        stderr.push_str(USAGE);
        return i32::from(args.is_empty());
    }
    let answer = answer(ctx, args);
    stdout.push_str(&answer.to_string());
    stdout.push('\n');
    match answer.get("ok") {
        Some(Value::Bool(true)) => 0,
        Some(_) | None => 1,
    }
}

fn answer(ctx: &CliContext, args: &[String]) -> Value {
    let named = |op: &Option<String>| op.as_ref().map(|op| op.replace('-', "_"));
    let line = match read_line(args) {
        Ok(line) => line,
        Err(error) => return request_refused(None, &error),
    };
    let stdin = || match &ctx.stdin_data {
        Some(data) => bounded(data.as_bytes()),
        None => bounded(std::io::stdin()),
    };
    let value = match request(&line, stdin) {
        Ok(value) => value,
        Err(error) => return request_refused(named(&line.op).as_deref(), &error),
    };
    // Read first: a request refused here builds nothing.
    let op = op_of(&value);
    let parsed = match parse(&value) {
        Ok(parsed) => parsed,
        Err(error) => return request_refused(op.as_deref(), &error),
    };
    let Some(build) = ctx.project_board else {
        return reason_refused(
            op.as_deref(),
            ErrorCode::StoreUnavailable,
            "the project board capability is not composed",
        );
    };
    let repo = line
        .repo
        .clone()
        .or_else(|| ctx.cwd.clone())
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let environment = ProjectBoardEnvironment {
        repo,
        remote: line.remote.clone(),
        access: match parsed.writes() {
            true => Access::Write,
            false => Access::Read,
        },
        vars: std::env::vars_os().collect(),
    };
    match build(&environment) {
        Ok(handles) => run(&handles.operations, &value),
        Err(BuildError::NoCommitter(why)) => {
            reason_refused(op.as_deref(), ErrorCode::NoCommitter, &why)
        }
        Err(BuildError::Unavailable(why)) => {
            reason_refused(op.as_deref(), ErrorCode::StoreUnavailable, &why)
        }
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
