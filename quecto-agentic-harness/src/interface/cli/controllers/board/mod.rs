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
//! `--board-id <16 hex digits>` and `--remote-url <url>` (#2705 B1) name
//! where the board is, as a caller that already knows (the pocket relay)
//! can: the id its records name it by and the remote's fetch URL. A read
//! takes each one given as given, so git is not asked for it; with both,
//! a read is composed with one git process: the check that `--remote` is
//! a remote the clone configures, with a URL of its own, so git never
//! takes the name for a folder (the request itself then reads the board
//! with git). A write takes neither on trust: git names the location, as
//! without them.
//!
//! `--url <url>` (#2705 B7) sends the request to a board server instead:
//! `unix:///<socket>`, or `http://<ip>:<port>` on loopback or a Tailscale
//! address, whose bearer token is read from `QUECTO_BOARD_TOKEN` or the
//! global config's `board.token`, never from the command line. Without
//! it, `QUECTO_BOARD_URL` or the global config's `board.url` names one;
//! with none, the board is the clone's, as before. The server writes as
//! `QUECTO_BOARD_COMMITTER` when it is set, else as the clone's committer.
//!
//! `--summary` (no value; `list`, `list_ready` and `list_takeable` only,
//! with flags or `--input` alike) gives each listed task its `summary`:
//! where its plan and latest run stand, read from one whole read of the
//! board (the pocket relay asks for it). Without it a listing reads and
//! answers exactly what it did before. A listing is never refused for its
//! summaries: when they cannot be read (the board kept moving, the index
//! disagrees with it, or its read failed), it is answered without them,
//! with `summary_skipped` naming why; a task one of whose plans or runs
//! the board holds twice goes without its own, naming why on its row. The
//! flag is the command line's own: the agent tool's request names no such
//! field.
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
//! keeps the scope to what the board uses it for. The identity is read
//! however many other settings come with it (the pocket relay sets 41),
//! in index order, so the last of a key wins as in git. A set git would
//! refuse (a count that is not empty or plain decimal 0 to 2147483647,
//! git's `INT_MAX`, or a setting below the count missing its key or
//! value) is dropped whole, and a write warns of it once on standard
//! error, naming the variable, never a key or a value. An environment
//! that sets
//! the scope for something else (a `safe.directory`, say) has those
//! settings ignored, as before. No other `GIT_*` variable reaches git.
//!
//! **Telemetry** (epic #2481 T3). While the owner's global config switches
//! the event log on, each call that answers writes one `board_op` record
//! (front end `cli`) to the day's `<base_dir>/audit/board-cli_<UTC
//! date>.jsonl`; with it off nothing is opened, timed or written. A caller
//! names itself in `QUECTO_BOARD_CLIENT`, read by allowlist: `pocket-relay`
//! is recorded as `pocket_relay`, any other value as `other`, and the
//! command line's own, unset, as `cli`.
use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::Value;

use crate::application::project_board::operations::ListSummaries;
use crate::application::project_board::telemetry::BoardRecording;
use crate::domain::project_board::value_objects::shown_text::shown_char;
use crate::domain::project_board::value_objects::telemetry::{
    BoardId, Client, FaultKind, FrontEnd,
};
use crate::interface::cli::CliContext;
use crate::interface::tools::project_board::recording::{BoardCall, CLIENT_VARIABLE, named_client};
use crate::interface::tools::project_board::request::{RequestError, fields_of};
use crate::interface::tools::project_board::site::{self, BoardSite};
use crate::interface::tools::project_board::{MAX_REQUEST_BYTES, command_line, json};

pub use crate::interface::tools::project_board::site::{
    Access, BuildError, KnownLocation, ProjectBoardEnvironment, ProjectBoardHandles,
    ProjectBoardHandlesBuilder,
};

/// Composition's builder of the agent `board` tool's recording (epic
/// #2481 T3): into the session's event log given, with the switch the
/// session decided when it started; `None` while it is off, or when the
/// log has no line to append through.
#[derive(Debug, Clone, Copy)]
pub struct BoardToolRecordingBuilder(
    pub  fn(
        bool,
        &crate::infrastructure::observability::audit_log::AuditLog,
    ) -> Option<crate::application::project_board::telemetry::BoardRecording>,
);

/// Composition's builder of `quecto board`'s recording: into the day's
/// `board-cli` log under the base directory given, only while the owner's
/// global config switches the event log on; `None` otherwise.
#[derive(Debug, Clone, Copy)]
pub struct CliRecordingBuilder(pub fn(&Path) -> Option<BoardRecording>);

/// Why `quecto board start` is refused (owner-approved, S4c).
const START_NEEDS_A_SESSION: &str =
    "start needs an agent session: ask a quecto agent, or use Pocket";

/// The ops `quecto board` takes, an allowlist: every op of the board's
/// table but `start`, which launches a swarm coordinator that only an
/// agent session can host (S4c). A new op is refused here until it is
/// listed.
const CLI_OPS: &[&str] = &[
    "bootstrap",
    "list",
    "list_ready",
    "list_takeable",
    "show",
    "create",
    "update_draft",
    "move",
    "renew",
    "take_over",
    "link_pr",
    "unlink_pr",
    "complete",
    "propose_plan",
    "edit_plan",
    "accept_plan",
    "discard_plan",
    "end_run",
    "clear_set_aside",
];

/// The ops `quecto board` takes, as its command line spells them
/// (`list-ready`): what `quecto help` lists, so it never drifts (#2660).
pub(in crate::interface::cli) fn command_line_ops() -> impl Iterator<Item = String> {
    CLI_OPS.iter().map(|op| op.replace('_', "-"))
}

/// Why `quecto board` does not take `op`, an op of the board's table it
/// does not list.
fn not_taken(op: &str) -> String {
    match op {
        "start" => START_NEEDS_A_SESSION.into(),
        other => format!("{other} is not taken by quecto board: ask a quecto agent"),
    }
}

/// The client a call is for, by allowlist, as `QUECTO_BOARD_CLIENT` names
/// it (`None`: unset, the command line's own): the value itself is never
/// recorded.
fn client(named: Option<&OsStr>) -> Client {
    named_client(named, Client::Cli)
}

const USAGE: &str = "usage: quecto board <op> [--<field> <value>]... [--input <json>|-] \
                     [--repo <path>] [--remote <name>] [--board-id <hex>] \
                     [--remote-url <url>] [--url <board server>] [--json] [--summary]\n";

/// The most bytes `--remote-url` takes.
const MAX_REMOTE_URL_BYTES: usize = 4096;

/// Why `--board-id` is refused.
const BOARD_ID_SHAPE: &str = "must be 16 lower-case hex digits";

/// Why `--remote-url` is refused.
const REMOTE_URL_SHAPE: &str = "must be at most 4096 bytes of shown characters and spaces";

/// `--remote-url`'s value, when it is one the command line takes: at most
/// [`MAX_REMOTE_URL_BYTES`] of shown characters and plain spaces (an
/// allowlist: no control or invisible character reaches git or a record).
fn remote_url(value: String) -> Option<String> {
    let shown = value.chars().all(|c| c == ' ' || shown_char(c));
    (shown && value.len() <= MAX_REMOTE_URL_BYTES).then_some(value)
}

/// The command line, read.
struct Line {
    op: Option<String>,
    fields: Vec<String>,
    input: Option<String>,
    repo: Option<PathBuf>,
    remote: String,
    /// `--board-id` and `--remote-url`: where the board is, if known.
    known: KnownLocation,
    /// `--summary`: a listing's tasks each with its summary.
    summary: bool,
}

/// A command line refused as it was read, and the op it named before the
/// refusal, if any (for the call's record; the answer names none).
#[derive(Debug)]
struct LineRefused {
    op: Option<String>,
    error: RequestError,
}

fn read_line(args: &[String]) -> Result<Line, LineRefused> {
    let mut line = Line {
        op: None,
        fields: Vec::new(),
        input: None,
        repo: None,
        remote: "origin".into(),
        known: KnownLocation::default(),
        summary: false,
    };
    match read_into(&mut line, args) {
        Ok(()) => Ok(line),
        Err(error) => Err(LineRefused { op: line.op, error }),
    }
}

/// Reads `args` into `line`, as far as they go.
fn read_into(line: &mut Line, args: &[String]) -> Result<(), RequestError> {
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
            // The one flag of the command line's own that takes no value.
            "--summary" => match line.summary {
                true => {
                    return Err(invalid(
                        "--summary",
                        "is given twice",
                        FaultKind::DuplicateKey,
                    ));
                }
                false => line.summary = true,
            },
            "--input" => line.input = Some(own("--input", rest.next())?),
            "--repo" => line.repo = Some(PathBuf::from(own("--repo", rest.next())?)),
            "--remote" => line.remote = own("--remote", rest.next())?,
            "--board-id" => {
                let given = own("--board-id", rest.next())?;
                let id = BoardId::of_hex(&given).ok_or_else(|| {
                    invalid("--board-id", BOARD_ID_SHAPE, FaultKind::InvalidValue)
                })?;
                line.known.board_id = Some(id);
            }
            "--remote-url" => {
                let given = own("--remote-url", rest.next())?;
                let url = remote_url(given).ok_or_else(|| {
                    invalid("--remote-url", REMOTE_URL_SHAPE, FaultKind::InvalidValue)
                })?;
                line.known.remote_url = Some(url);
            }
            "--url" => {
                let given = own("--url", rest.next())?;
                let url = remote_url(given)
                    .ok_or_else(|| invalid("--url", REMOTE_URL_SHAPE, FaultKind::InvalidValue))?;
                line.known.url = Some(url);
            }
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
    Ok(())
}

/// The request the command line names; `stdin` reads `--input -`.
fn request(
    line: &Line,
    stdin: impl FnOnce() -> Result<String, InputError>,
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
                    RequestError::whole("input", problem, error.fault())
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

/// Why `--input -` could not be read as a request's text.
#[derive(Debug)]
enum InputError {
    /// More than [`MAX_REQUEST_BYTES`].
    TooLarge,
    /// Not UTF-8.
    NotUtf8,
    /// Standard input could not be read.
    Read(std::io::Error),
}

impl std::fmt::Display for InputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge => write!(f, "it is more than {MAX_REQUEST_BYTES} bytes"),
            Self::NotUtf8 => f.write_str("it is not UTF-8"),
            Self::Read(error) => write!(f, "{error}"),
        }
    }
}

impl InputError {
    /// How the request's text was wrong, as the shared path records it.
    fn fault(&self) -> FaultKind {
        match self {
            Self::TooLarge => FaultKind::TooLarge,
            Self::NotUtf8 => FaultKind::NotJson,
            Self::Read(_) => FaultKind::InvalidValue,
        }
    }
}

/// At most [`MAX_REQUEST_BYTES`] of `input`, as text.
fn bounded(input: impl Read) -> Result<String, InputError> {
    let mut bytes = Vec::new();
    let limit = u64::try_from(MAX_REQUEST_BYTES).expect("the request bound fits a u64");
    input
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(InputError::Read)?;
    if bytes.len() > MAX_REQUEST_BYTES {
        return Err(InputError::TooLarge);
    }
    String::from_utf8(bytes).map_err(|_| InputError::NotUtf8)
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
    let recording = ctx
        .board_cli_recording
        .and_then(|build| (build.0)(&ctx.base_dir()))
        .map(Arc::new);
    let named = std::env::var_os(CLIENT_VARIABLE);
    let mut call = BoardCall::start(recording, FrontEnd::Cli, client(named.as_deref()));
    let answer = answer(ctx, args, &mut call);
    stdout.push_str(&answer.to_string());
    stdout.push('\n');
    call.finish(&answer, false);
    match answer.get("ok") {
        Some(Value::Bool(true)) => 0,
        Some(_) | None => 1,
    }
}

fn answer(ctx: &CliContext, args: &[String], call: &mut BoardCall) -> Value {
    // An op is echoed back only when it is one: never a stranger's text.
    let named = |op: &Option<String>| {
        op.as_ref()
            .map(|op| op.replace('-', "_"))
            .filter(|op| fields_of(op).is_some())
    };
    let line = match read_line(args) {
        Ok(line) => line,
        Err(LineRefused { op, error }) => {
            call.named(named(&op).as_deref());
            return call.refuse_request(None, &error);
        }
    };
    let stdin = || match &ctx.stdin_data {
        Some(data) => bounded(data.as_bytes()),
        None => bounded(std::io::stdin()),
    };
    let value = match request(&line, stdin) {
        Ok(value) => value,
        Err(error) => {
            let op = named(&line.op);
            call.named(op.as_deref());
            return call.refuse_request(op.as_deref(), &error);
        }
    };
    // Only the ops the command line takes (an allowlist) go on: one it
    // does not, `start` (S4c, owner-approved words), is refused here, as
    // the request it is, before anything is built. A request naming no op
    // of the table is the shared parser's to refuse.
    let op = value
        .get("op")
        .and_then(Value::as_str)
        .filter(|op| fields_of(op).is_some());
    match op {
        Some(op) if CLI_OPS.contains(&op) => {}
        None => {}
        Some(op) => {
            call.named(Some(op));
            let refusal = RequestError::of("op", not_taken(op), FaultKind::InvalidValue);
            return call.refuse_request(Some(op), &refusal);
        }
    }
    let repo = line
        .repo
        .clone()
        .or_else(|| ctx.cwd.clone())
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let site = BoardSite {
        repo,
        remote: line.remote.clone(),
        known: line.known.clone(),
        build: ctx.project_board,
        launcher: None,
        swarm_session: None,
    };
    let summaries = match line.summary {
        true => ListSummaries::Wanted,
        false => ListSummaries::Omitted,
    };
    site::answer_with(&site, &value.to_string(), None, summaries, call)
}

#[cfg(test)]
mod tests;
