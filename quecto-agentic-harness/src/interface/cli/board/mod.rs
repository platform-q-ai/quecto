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
//! is the real control. The identity is `user.name` and `user.email` as
//! `git config` resolves them: `GIT_AUTHOR_*` and `GIT_COMMITTER_*` in
//! the environment are not honoured.
//!
//! **Command-scope identity.** A caller that writes as a bot (the pocket
//! relay, quecto's container scripts) sets git's command-scope
//! configuration, which outranks the clone's own `.git/config` key by
//! key: `GIT_CONFIG_COUNT=2 GIT_CONFIG_KEY_0=user.name
//! GIT_CONFIG_VALUE_0='quecto pocket' GIT_CONFIG_KEY_1=user.email
//! GIT_CONFIG_VALUE_1=pocket@quecto.invalid`. A set naming only one of
//! `user.name` and `user.email` leaves the other to the clone, and a
//! write warns that it does. Of the set only the `user.name` and
//! `user.email` settings reach git (matched in any case, as git matches
//! them, and passed in lower case), renumbered; every other setting
//! (`core.sshCommand` or `credential.helper`, which run programs;
//! `author.*`, which the board would override; a key git would refuse,
//! such as an empty one) is skipped, quietly, since a key can carry a
//! credential. The caller could already reach those keys through
//! `GIT_CONFIG_GLOBAL`, so skipping them withholds no power it lacks; it
//! keeps the scope to what the board uses it for. The set is read more
//! strictly than git reads it: a count that is not empty or plain decimal
//! 0 to 16 (quecto's own cap), or a setting missing its key or value,
//! drops the set whole, and a write warns of it once on standard error,
//! naming the variable, never a key or a value. An environment that sets
//! the scope for something else (a `safe.directory`, say) has those
//! settings ignored, as before. No other `GIT_*` variable reaches git.
use std::io::Read;
use std::path::PathBuf;

use serde_json::Value;

use super::CliContext;
use crate::domain::project_board::value_objects::telemetry::FaultKind;
use crate::interface::tools::project_board::answer::request_refused;
use crate::interface::tools::project_board::request::{RequestError, fields_of};
use crate::interface::tools::project_board::site::{self, BoardSite};
use crate::interface::tools::project_board::{MAX_REQUEST_BYTES, command_line, json};

pub use crate::interface::tools::project_board::site::{
    Access, BuildError, ProjectBoardEnvironment, ProjectBoardHandles, ProjectBoardHandlesBuilder,
};

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
    let invalid = |flag: &str, problem: &str, fault| {
        RequestError::of(flag.trim_start_matches('-'), problem, fault)
    };
    // A flag of the command line's own: given once, with a value.
    let mut given: Vec<&str> = Vec::new();
    let mut own = |flag: &'static str, value: Option<&String>| {
        let value = value
            .filter(|value| !value.is_empty())
            .ok_or_else(|| invalid(flag, "needs a value", FaultKind::Missing))?;
        match given.contains(&flag) {
            true => Err(invalid(flag, "is given twice", FaultKind::DuplicateKey)),
            false => {
                given.push(flag);
                Ok(value.clone())
            }
        }
    };
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--json" => {}
            "--input" => line.input = Some(own("--input", rest.next())?),
            "--repo" => line.repo = Some(PathBuf::from(own("--repo", rest.next())?)),
            "--remote" => line.remote = own("--remote", rest.next())?,
            flag if flag.starts_with("--") => {
                // An unknown flag is never echoed back as a field.
                let value = rest.next().ok_or_else(|| {
                    invalid("flags", "a --<field> needs a value", FaultKind::Missing)
                })?;
                line.fields.extend([flag.to_string(), value.clone()]);
            }
            op if line.op.is_none() => line.op = Some(op.to_string()),
            _ => {
                // A fixed field: the stray text is never echoed back.
                return Err(RequestError::stray(
                    "arguments",
                    "hold one op, then --<field> <value> pairs",
                    1,
                ));
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
    let invalid = |field: &str, problem: &str, fault| RequestError::of(field, problem, fault);
    match (&line.input, &line.op) {
        (Some(_), _) if !line.fields.is_empty() => Err(RequestError::whole(
            "input",
            "is the whole request: give fields there, not as flags",
            FaultKind::InvalidValue,
        )),
        (Some(input), op) => {
            let text = match input.as_str() {
                "-" => stdin().map_err(|error| {
                    let problem = format!("could not be read: {error}");
                    invalid("input", &problem, FaultKind::InvalidValue)
                })?,
                text => text.to_string(),
            };
            let mut value = json::strict(&text).map_err(|error| {
                RequestError::whole("input", format!("must be JSON: {error}"), error.fault())
            })?;
            let object = value.as_object_mut().ok_or_else(|| {
                RequestError::whole("input", "must be a JSON object", FaultKind::WrongType)
            })?;
            if let Some(op) = op {
                let op = Value::String(op.replace('-', "_"));
                match object.get("op") {
                    None => {
                        object.insert("op".into(), op);
                    }
                    Some(named) if *named == op => {}
                    Some(_) => {
                        let fault = FaultKind::InvalidValue;
                        return Err(invalid("op", "differs from the input's op", fault));
                    }
                }
            }
            Ok(value)
        }
        (None, Some(op)) => command_line::request(op, &line.fields),
        (None, None) => Err(invalid("op", "is required", FaultKind::Missing)),
    }
}

/// At most [`MAX_REQUEST_BYTES`] of `input`, as text.
fn bounded(input: impl Read) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    let limit = u64::try_from(MAX_REQUEST_BYTES).expect("the request bound fits a u64");
    input.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() > MAX_REQUEST_BYTES {
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
    // An op is echoed back only when it is one: never a stranger's text.
    let named = |op: &Option<String>| {
        op.as_ref()
            .map(|op| op.replace('-', "_"))
            .filter(|op| fields_of(op).is_some())
    };
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
    let repo = line
        .repo
        .clone()
        .or_else(|| ctx.cwd.clone())
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let site = BoardSite {
        repo,
        remote: line.remote.clone(),
        build: ctx.project_board,
    };
    site::answer(&site, &value.to_string(), None)
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
