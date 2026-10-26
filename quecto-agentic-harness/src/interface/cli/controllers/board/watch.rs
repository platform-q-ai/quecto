//! `quecto board watch` (epic #2705 B10): a board server's change feed,
//! one JSON line per event, for scripts and the Pocket relay's tests: each
//! revision, reset and change of epoch (an epoch already printed is not
//! printed again as a stream reopens), which `--count` counts. It runs
//! until `--count` lines are printed, its output is closed, or the watch
//! cannot go on (exit 1). A cut stream is opened again from the last
//! event, so each revision is printed once, with no gap. Once its output
//! has no reader left it is not opened again. Without `--count`, output
//! not written to the terminal as it goes is held to [`MAX_HELD_BYTES`].
use std::path::PathBuf;

use crate::application::project_board::ports::feed::WatchFlow;
use crate::application::project_board::use_cases::WatchBoard;
use crate::domain::project_board::value_objects::telemetry::BoardId;
use crate::interface::cli::CliContext;
use crate::interface::tools::project_board::feed_events::event_json;
use crate::interface::tools::project_board::site::{
    Access, BuildError, KnownLocation, ProjectBoardEnvironment,
};

pub const USAGE: &str = "usage: quecto board watch [--url <board server>] [--board-id <hex>] \
                         [--repo <path>] [--remote <name>] [--after <revision>] [--count <n>]\n";

/// The most output held, not written as it goes (a run whose output is
/// returned whole), before the watch stops: 1 MiB.
pub const MAX_HELD_BYTES: usize = 1024 * 1024;

/// A watch, and whether its output still has a reader.
pub struct BoardWatchHandles {
    pub watch: WatchBoard,
    pub output_open: fn() -> bool,
}

/// Composition's builder of a watch of the server the environment names.
pub type BoardWatchBuilder = fn(&ProjectBoardEnvironment) -> Result<BoardWatchHandles, BuildError>;

/// The command line, read.
#[derive(Debug, Default, PartialEq, Eq)]
struct Options {
    url: Option<String>,
    board_id: Option<BoardId>,
    repo: Option<PathBuf>,
    remote: Option<String>,
    after: Option<u64>,
    count: Option<u64>,
}

/// `args` (after `watch`), each flag of the allowlist once, with a value.
fn parse(args: &[String]) -> Result<Options, String> {
    let mut options = Options::default();
    let mut rest = args.iter();
    while let Some(flag) = rest.next() {
        let value = rest
            .next()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("{flag} needs a value"))?;
        let number = |what: &str| {
            let digits =
                (1..=20).contains(&value.len()) && value.bytes().all(|b| b.is_ascii_digit());
            digits
                .then(|| value.parse::<u64>().ok())
                .flatten()
                .ok_or_else(|| format!("{flag} must be {what}"))
        };
        let twice = match flag.as_str() {
            "--url" => options.url.replace(value.clone()).is_some(),
            "--board-id" => {
                let id = BoardId::of_hex(value)
                    .ok_or_else(|| "--board-id must be 16 lower-case hex digits".to_string())?;
                options.board_id.replace(id).is_some()
            }
            "--repo" => options.repo.replace(PathBuf::from(value)).is_some(),
            "--remote" => options.remote.replace(value.clone()).is_some(),
            "--after" => options
                .after
                .replace(number("a revision number")?)
                .is_some(),
            "--count" => {
                let count = number("a number of events, 1 or more")?;
                match count {
                    0 => return Err("--count must be a number of events, 1 or more".into()),
                    _ => options.count.replace(count).is_some(),
                }
            }
            _ => {
                return Err(
                    "an unknown option: the options are --url, --board-id, --repo, \
                             --remote, --after and --count"
                        .into(),
                );
            }
        };
        if twice {
            return Err(format!("{flag} is given twice"));
        }
    }
    Ok(options)
}

/// `quecto board watch <args>`: 0 once `--count` events are printed or
/// the output closes; 1 when the watch cannot start or go on.
pub fn run(ctx: &CliContext, args: &[String], stdout: &mut String, stderr: &mut String) -> i32 {
    let options = match parse(args) {
        Ok(options) => options,
        Err(why) => {
            stderr.push_str(&format!("board watch: {why}\n{USAGE}"));
            return 1;
        }
    };
    let Some(build) = ctx.board_watch else {
        stderr.push_str("board watch: the board watch is not composed\n");
        return 1;
    };
    let repo = options
        .repo
        .clone()
        .or_else(|| ctx.cwd.clone())
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let environment = ProjectBoardEnvironment {
        repo,
        remote: options.remote.clone().unwrap_or_else(|| "origin".into()),
        known: KnownLocation {
            board_id: options.board_id.clone(),
            remote_url: None,
            url: options.url.clone(),
        },
        access: Access::Read,
        vars: std::env::vars_os().collect(),
        cancelled: None,
        launcher: None,
        swarm_session: None,
    };
    let handles = match build(&environment) {
        Ok(handles) => handles,
        Err(
            BuildError::Unavailable(why)
            | BuildError::NoSuchRemote(why)
            | BuildError::NoCommitter(why),
        ) => {
            stderr.push_str(&format!("board watch: {why}\n"));
            return 1;
        }
    };
    match ctx.live_output {
        true => {
            let mut out = std::io::stdout().lock();
            let output = Output::Live(&mut out);
            watch_into(&handles, options.after, options.count, output, stderr)
        }
        false => watch_into(
            &handles,
            options.after,
            options.count,
            Output::Held(stdout),
            stderr,
        ),
    }
}

/// Where a watch's lines go: written as they come (to standard output), or
/// held to be returned whole.
pub(super) enum Output<'o> {
    Live(&'o mut dyn std::io::Write),
    Held(&'o mut String),
}

/// How a line's write went.
enum Wrote {
    Yes,
    /// The output's reader has gone: the watch ends, quietly.
    ReaderGone,
    /// Any other failure of the output: the watch ends, saying so.
    Failed(std::io::Error),
    /// Held output reached its bound with no `--count`.
    HeldFull,
}

/// Runs `handles`' watch from `after`, writing each event's line to
/// `output` until `count` lines (if given): 0 when it ends as it should,
/// 1 saying why on `stderr` when it cannot go on.
pub(super) fn watch_into(
    handles: &BoardWatchHandles,
    after: Option<u64>,
    count: Option<u64>,
    mut output: Output<'_>,
    stderr: &mut String,
) -> i32 {
    // Written as it goes, the output has a reader while standard output
    // does; held, always.
    let live = matches!(output, Output::Live(_));
    let output_open = handles.output_open;
    let mut open = || match live {
        true => output_open(),
        false => true,
    };
    let (mut printed, mut ended) = (0u64, Wrote::Yes);
    let watched = handles.watch.watch(after, &mut open, &mut |event| {
        let line = format!("{}\n", event_json(&event));
        let wrote = match &mut output {
            Output::Live(out) => match out.write_all(line.as_bytes()).and_then(|()| out.flush()) {
                Ok(()) => Wrote::Yes,
                // A reader gone (an allowlist: a broken pipe alone) ends it
                // quietly; any other failure is said.
                Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Wrote::ReaderGone,
                Err(error) => Wrote::Failed(error),
            },
            // Bounded only with no --count: a count bounds it already.
            Output::Held(held) => match (count, held.len() + line.len() <= MAX_HELD_BYTES) {
                (Some(_), _) | (None, true) => {
                    held.push_str(&line);
                    Wrote::Yes
                }
                (None, false) => Wrote::HeldFull,
            },
        };
        printed += 1;
        match (wrote, count) {
            (Wrote::Yes, Some(count)) if printed >= count => WatchFlow::Stop,
            (Wrote::Yes, Some(_) | None) => WatchFlow::Continue,
            (stopped, _) => {
                ended = stopped;
                WatchFlow::Stop
            }
        }
    });
    match (watched, ended) {
        (Ok(()), Wrote::Yes | Wrote::ReaderGone) => 0,
        (Ok(()), Wrote::HeldFull) => {
            let most = MAX_HELD_BYTES / 1024;
            stderr.push_str(&format!(
                "board watch: stopped at {most} KiB of output held; give --count to bound it\n"
            ));
            1
        }
        (Ok(()), Wrote::Failed(error)) => {
            stderr.push_str(&format!("board watch: output failed: {error}\n"));
            1
        }
        (Err(why), _) => {
            stderr.push_str(&format!("board watch: {why}\n"));
            1
        }
    }
}

#[cfg(test)]
#[path = "watch_tests.rs"]
mod tests;
