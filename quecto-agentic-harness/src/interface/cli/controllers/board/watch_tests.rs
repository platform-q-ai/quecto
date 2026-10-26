//! `quecto board watch` (epic #2705 B10): one JSON line per event.
use super::*;
use crate::application::project_board::ports::feed::{
    BoardRevision, BoardStream, BoardStreams, FeedEvent, OpenRefused, StreamRead, WatchPauses,
};

/// One stream: an epoch, then revisions from `after` on, `left` of them,
/// then a malformed read (or, with none left to count, revisions for ever).
struct Counting {
    next: u64,
    left: Option<u64>,
    told_epoch: bool,
}

impl BoardStream for Counting {
    fn read(&mut self) -> StreamRead {
        if !self.told_epoch {
            self.told_epoch = true;
            return StreamRead::Event {
                id: self.next,
                event: FeedEvent::Epoch { epoch: 1 },
            };
        }
        match &mut self.left {
            Some(0) => return StreamRead::Malformed("the server went away".into()),
            Some(left) => *left -= 1,
            None => {}
        }
        self.next += 1;
        let revision = BoardRevision {
            epoch: 1,
            revision: self.next,
        };
        StreamRead::Event {
            id: self.next,
            event: FeedEvent::Revision(revision),
        }
    }

    fn retry(&self) -> Option<std::time::Duration> {
        None
    }
}

/// Streams of [`Counting`]: 99 revisions, or 60,000 (about 3 MiB of lines,
/// past the held bound; never endless, so a missing bound fails, never
/// fills the machine).
struct Streams(Option<u64>);

impl BoardStreams for Streams {
    fn open(&self, after: Option<u64>) -> Result<Box<dyn BoardStream>, OpenRefused> {
        Ok(Box::new(Counting {
            next: after.unwrap_or(0),
            left: self.0,
            told_epoch: false,
        }))
    }
}

struct NoPauses;

impl WatchPauses for NoPauses {
    fn pause(&self, _: std::time::Duration) {}
}

fn watch_of(streams: Streams) -> BoardWatchHandles {
    let timing = crate::application::project_board::use_cases::WatchTiming::PRODUCTION;
    BoardWatchHandles {
        watch: WatchBoard::new(Box::new(streams), std::sync::Arc::new(NoPauses), timing),
        output_open: || true,
    }
}

fn counting(environment: &ProjectBoardEnvironment) -> Result<BoardWatchHandles, BuildError> {
    assert_eq!(environment.access, Access::Read, "a watch writes nothing");
    match environment.known.url.as_deref() {
        Some("unix:///tmp/board.sock") => Ok(watch_of(Streams(Some(99)))),
        Some("unix:///tmp/endless.sock") => Ok(watch_of(Streams(Some(60_000)))),
        _ => Err(BuildError::Unavailable("watch needs a board server".into())),
    }
}

fn watched(args: &[&str]) -> (i32, String, String) {
    let ctx = CliContext {
        board_watch: Some(counting),
        ..CliContext::default()
    };
    let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
    let (mut stdout, mut stderr) = (String::new(), String::new());
    let code = run(&ctx, &args, &mut stdout, &mut stderr);
    (code, stdout, stderr)
}

#[test]
fn each_event_is_one_json_line_until_the_count() {
    let (code, stdout, stderr) = watched(&[
        "--url",
        "unix:///tmp/board.sock",
        "--after",
        "41",
        "--count",
        "3",
    ]);
    assert_eq!((code, stderr.as_str()), (0, ""));
    let lines: Vec<serde_json::Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(
        lines,
        vec![
            serde_json::json!({"kind": "epoch", "epoch": 1}),
            serde_json::json!({"kind": "revision", "epoch": 1, "revision": 42}),
            serde_json::json!({"kind": "revision", "epoch": 1, "revision": 43}),
        ]
    );
}

#[test]
fn a_watch_that_cannot_go_on_exits_1_saying_why() {
    let (code, _, stderr) = watched(&["--url", "unix:///tmp/board.sock"]);
    assert_eq!(
        (code, stderr.as_str()),
        (1, "board watch: the server went away\n")
    );
    let (code, _, stderr) = watched(&[]);
    assert_eq!(
        (code, stderr.as_str()),
        (1, "board watch: watch needs a board server\n")
    );
}

#[test]
fn the_options_are_an_allowlist() {
    for (args, why) in [
        (&["--title", "x"][..], "an unknown option"),
        (&["--after", "-1"][..], "--after must be a revision number"),
        (
            &["--count", "0"][..],
            "--count must be a number of events, 1 or more",
        ),
        (&["--url", "a", "--url", "b"][..], "--url is given twice"),
        (
            &["--board-id", "XYZ"][..],
            "--board-id must be 16 lower-case hex digits",
        ),
        (&["--after"][..], "--after needs a value"),
    ] {
        let (code, stdout, stderr) = watched(args);
        assert_eq!((code, stdout.as_str()), (1, ""), "{args:?}");
        assert!(
            stderr.starts_with(&format!("board watch: {why}")),
            "{args:?}: {stderr}"
        );
    }
}

#[test]
fn quecto_board_watch_is_dispatched_before_any_op() {
    let ctx = CliContext {
        board_watch: Some(counting),
        ..CliContext::default()
    };
    let args: Vec<String> = ["watch", "--url", "unix:///tmp/board.sock", "--count", "1"]
        .iter()
        .map(|arg| arg.to_string())
        .collect();
    let (mut stdout, mut stderr) = (String::new(), String::new());
    let code = super::super::cmd_board(&ctx, &args, &mut stdout, &mut stderr);
    assert_eq!(
        (code, stdout.as_str()),
        (0, "{\"kind\":\"epoch\",\"epoch\":1}\n"),
        "{stderr}"
    );
}

/// Review (#2757 round 3, nit): with no --count, output held to be returned
/// whole is bounded: the watch stops at MAX_HELD_BYTES, saying so.
#[test]
fn held_output_without_a_count_is_bounded() {
    let (code, stdout, stderr) = watched(&["--url", "unix:///tmp/endless.sock"]);
    assert_eq!(code, 1);
    assert!(stdout.len() <= MAX_HELD_BYTES, "{}", stdout.len());
    assert!(stdout.len() > MAX_HELD_BYTES - 1024, "{}", stdout.len());
    assert!(stdout.ends_with('\n'), "whole lines only");
    assert_eq!(
        stderr,
        "board watch: stopped at 1024 KiB of output held; give --count to bound it\n"
    );
}

/// Review (#2757 round 4, L-b): the held bound is for a watch with no
/// --count; one whose count runs past the bound's lines completes.
#[test]
fn a_count_past_the_held_bound_completes() {
    let count = "30000";
    let (code, stdout, stderr) = watched(&["--url", "unix:///tmp/endless.sock", "--count", count]);
    assert_eq!((code, stderr.as_str()), (0, ""));
    assert_eq!(stdout.lines().count(), 30_000);
    assert!(
        stdout.len() > MAX_HELD_BYTES,
        "past the bound: {}",
        stdout.len()
    );
}

/// A writer that fails every write with `kind`.
struct Failing(std::io::ErrorKind);

impl std::io::Write for Failing {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::new(self.0, "the output failed"))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Live output to `writer`, over 99 revisions; its exit and stderr.
fn live_into(writer: &mut dyn std::io::Write, output_open: fn() -> bool) -> (i32, String) {
    let handles = BoardWatchHandles {
        output_open,
        ..watch_of(Streams(Some(99)))
    };
    let mut stderr = String::new();
    let code = watch_into(&handles, None, Some(5), Output::Live(writer), &mut stderr);
    (code, stderr)
}

/// Review (#2757 round 4, L-a): only a reader gone (a broken pipe) ends
/// the watch quietly, exit 0; any other failure of the output is exit 1,
/// saying so.
#[test]
fn only_a_broken_pipe_ends_the_watch_quietly() {
    let (code, stderr) = live_into(&mut Failing(std::io::ErrorKind::BrokenPipe), || true);
    assert_eq!((code, stderr.as_str()), (0, ""));
    let (code, stderr) = live_into(&mut Failing(std::io::ErrorKind::StorageFull), || true);
    assert_eq!(code, 1);
    assert_eq!(stderr, "board watch: output failed: the output failed\n");
}

static ASKED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn gone() -> bool {
    ASKED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    false
}

fn never_asked() -> bool {
    panic!("held output never asks whether standard output has a reader")
}

/// Review (#2757 round 4): whether standard output has a reader is asked
/// only of live output; held output never asks it.
#[test]
fn only_live_output_asks_for_a_reader() {
    let mut written = Vec::new();
    let (code, stderr) = live_into(&mut written, gone);
    assert_eq!((code, stderr.as_str()), (0, ""));
    assert!(
        written.is_empty(),
        "no reader: nothing opened, nothing written"
    );
    assert!(ASKED.load(std::sync::atomic::Ordering::SeqCst) >= 1);
    let handles = BoardWatchHandles {
        output_open: never_asked,
        ..watch_of(Streams(Some(99)))
    };
    let (mut held, mut stderr) = (String::new(), String::new());
    let code = watch_into(
        &handles,
        None,
        Some(3),
        Output::Held(&mut held),
        &mut stderr,
    );
    assert_eq!((code, held.lines().count()), (0, 3), "{stderr}");
}
