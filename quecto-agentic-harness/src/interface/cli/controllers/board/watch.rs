//! `quecto board watch` (epic #2705 B10): a board server's change feed,
//! one JSON line per event, for scripts and the Pocket relay's tests: each
//! revision, reset and change of epoch (an epoch already printed is not
//! printed again as a stream reopens), which `--count` counts. It runs
//! until `--count` lines are printed, its output is closed, or the watch
//! cannot go on (exit 1). A cut stream is opened again from the last
//! event, so each revision is printed once, with no gap.
use std::path::PathBuf;

use crate::application::project_board::ports::feed::{BoardWatch, WatchFlow};
use crate::domain::project_board::value_objects::telemetry::BoardId;
use crate::interface::cli::CliContext;
use crate::interface::cli::board_server::feed::event_json;
use crate::interface::tools::project_board::site::{
    Access, BuildError, KnownLocation, ProjectBoardEnvironment,
};

pub const USAGE: &str = "usage: quecto board watch [--url <board server>] [--board-id <hex>] \
                         [--repo <path>] [--remote <name>] [--after <revision>] [--count <n>]\n";

/// Composition's builder of a watch of the server the environment names.
pub type BoardWatchBuilder =
    fn(&ProjectBoardEnvironment) -> Result<Box<dyn BoardWatch>, BuildError>;

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
    let watch = match build(&environment) {
        Ok(watch) => watch,
        Err(
            BuildError::Unavailable(why)
            | BuildError::NoSuchRemote(why)
            | BuildError::NoCommitter(why),
        ) => {
            stderr.push_str(&format!("board watch: {why}\n"));
            return 1;
        }
    };
    let (live, mut printed) = (ctx.live_output, 0u64);
    let watched = watch.watch(options.after, &mut |event| {
        let line = format!("{}\n", event_json(&event));
        // A watch prints as it goes: a closed output ends it.
        let written = match live {
            true => {
                use std::io::Write;
                let mut out = std::io::stdout().lock();
                out.write_all(line.as_bytes())
                    .and_then(|()| out.flush())
                    .is_ok()
            }
            false => {
                stdout.push_str(&line);
                true
            }
        };
        printed += 1;
        match (written, options.count) {
            (true, Some(count)) if printed >= count => WatchFlow::Stop,
            (true, Some(_) | None) => WatchFlow::Continue,
            (false, _) => WatchFlow::Stop,
        }
    });
    match watched {
        Ok(()) => 0,
        Err(why) => {
            stderr.push_str(&format!("board watch: {why}\n"));
            1
        }
    }
}

#[cfg(test)]
#[path = "watch_tests.rs"]
mod tests;
