//! `quecto board watch` (epic #2705 B10): one JSON line per event.
use super::*;
use crate::application::project_board::ports::feed::{BoardRevision, FeedEvent};

/// Tells an epoch then revisions 1, 2, 3... until stopped, from `after`.
struct Counting;

impl BoardWatch for Counting {
    fn watch(
        &self,
        after: Option<u64>,
        each: &mut dyn FnMut(FeedEvent) -> WatchFlow,
    ) -> Result<(), String> {
        let start = after.unwrap_or(0);
        if each(FeedEvent::Epoch { epoch: 1 }) == WatchFlow::Stop {
            return Ok(());
        }
        for revision in start + 1..start + 100 {
            let event = FeedEvent::Revision(BoardRevision { epoch: 1, revision });
            if each(event) == WatchFlow::Stop {
                return Ok(());
            }
        }
        Err("the server went away".into())
    }
}

fn counting(environment: &ProjectBoardEnvironment) -> Result<Box<dyn BoardWatch>, BuildError> {
    assert_eq!(environment.access, Access::Read, "a watch writes nothing");
    match environment.known.url.as_deref() {
        Some("unix:///tmp/board.sock") => Ok(Box::new(Counting)),
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
