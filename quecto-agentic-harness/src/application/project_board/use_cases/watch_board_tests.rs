//! A watch's policy (epic #2705 B10) over scripted opens and recorded
//! pauses: reopens, epochs, refusals, and a busy server's backoff.
use std::collections::VecDeque;
use std::sync::Mutex;

use super::*;
use crate::application::project_board::ports::feed::{BoardRevision, BoardStream};

/// Opens, or pauses, past which a watch is a runaway.
const RUNAWAY: usize = 10_000;

/// What one open gives.
#[derive(Debug, Clone)]
enum Open {
    Busy,
    Refused,
    Unreachable,
    Stream(Vec<StreamRead>, Option<Duration>),
}

/// Opens as scripted (the last repeats for ever), each cursor asked.
struct Scripted {
    script: Mutex<VecDeque<Open>>,
    asked: Arc<Mutex<Vec<Option<u64>>>>,
}

struct Played(VecDeque<StreamRead>, Option<Duration>);

impl BoardStream for Played {
    fn read(&mut self) -> StreamRead {
        self.0.pop_front().unwrap_or(StreamRead::Ended)
    }

    fn retry(&self) -> Option<Duration> {
        self.1
    }
}

impl BoardStreams for Scripted {
    fn open(&self, after: Option<u64>) -> Result<Box<dyn BoardStream>, OpenRefused> {
        let mut asked = self.asked.lock().unwrap();
        // A runaway watch fails here, never fills the machine.
        assert!(asked.len() < RUNAWAY, "the watch opened {RUNAWAY} streams");
        asked.push(after);
        drop(asked);
        let mut script = self.script.lock().unwrap();
        let open = match script.len() {
            1 => script[0].clone(),
            _ => script.pop_front().expect("a scripted open"),
        };
        match open {
            Open::Busy => Err(OpenRefused::Busy("busy".into())),
            Open::Refused => Err(OpenRefused::Refused("refused (503 unavailable)".into())),
            Open::Unreachable => Err(OpenRefused::Unreachable("no server".into())),
            Open::Stream(reads, retry) => Ok(Box::new(Played(reads.into(), retry))),
        }
    }
}

#[derive(Default)]
struct Recorded(Mutex<Vec<Duration>>);

impl WatchPauses for Recorded {
    fn pause(&self, how_long: Duration) {
        let mut paused = self.0.lock().unwrap();
        assert!(paused.len() < RUNAWAY, "the watch paused {RUNAWAY} times");
        paused.push(how_long);
    }
}

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

/// Fast times: no reopen pause, busy 100 ms to 10 s for a minute or 64 tries.
const FAST: WatchTiming = WatchTiming {
    pause: Duration::ZERO,
    min_backoff: Duration::from_millis(100),
    max_backoff: Duration::from_secs(10),
    busy_for: Duration::from_secs(60),
    busy_tries: 64,
    attempts: 5,
};

struct Run {
    told: Vec<FeedEvent>,
    result: Result<(), String>,
    asked: Vec<Option<u64>>,
    paused: Vec<Duration>,
}

/// A watch of `script` under `timing` from cursor 0, stopped at revision
/// `last`, its output open for `open_for` asks.
fn run(script: Vec<Open>, timing: WatchTiming, last: u64, open_for: usize) -> Run {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let streams = Scripted {
        script: Mutex::new(script.into()),
        asked: Arc::clone(&asked),
    };
    let pauses = Arc::new(Recorded::default());
    let watch = WatchBoard::new(Box::new(streams), Arc::clone(&pauses) as _, timing);
    let mut told = Vec::new();
    let mut asks = 0;
    let result = watch.watch(
        Some(0),
        &mut || {
            asks += 1;
            asks <= open_for
        },
        &mut |event| {
            told.push(event);
            match event {
                FeedEvent::Revision(BoardRevision { revision, .. }) if revision == last => {
                    WatchFlow::Stop
                }
                _ => WatchFlow::Continue,
            }
        },
    );
    let asked = asked.lock().unwrap().clone();
    let paused = pauses.0.lock().unwrap().clone();
    Run {
        told,
        result,
        asked,
        paused,
    }
}

fn revision(id: u64) -> StreamRead {
    StreamRead::Event {
        id,
        event: FeedEvent::Revision(BoardRevision {
            epoch: 1,
            revision: id,
        }),
    }
}

fn epoch(id: u64, epoch: u64) -> StreamRead {
    StreamRead::Event {
        id,
        event: FeedEvent::Epoch { epoch },
    }
}

fn stream(reads: Vec<StreamRead>) -> Open {
    Open::Stream(reads, None)
}

const OPEN: usize = usize::MAX;

#[test]
fn an_ended_stream_is_opened_again_from_its_last_id_and_an_epoch_is_told_once() {
    let script = vec![
        stream(vec![
            epoch(0, 1),
            revision(1),
            revision(2),
            StreamRead::Ended,
        ]),
        stream(vec![epoch(2, 1), epoch(2, 2), revision(3)]),
    ];
    let run = run(script, FAST, 3, OPEN);
    assert_eq!(run.result, Ok(()));
    assert_eq!(run.asked, vec![Some(0), Some(2)]);
    let epochs: Vec<_> = run
        .told
        .iter()
        .filter(|event| matches!(event, FeedEvent::Epoch { .. }))
        .collect();
    assert_eq!(
        epochs,
        [
            &FeedEvent::Epoch { epoch: 1 },
            &FeedEvent::Epoch { epoch: 2 }
        ]
    );
}

#[test]
fn a_reset_is_handed_on_and_read_on_from() {
    let reset = StreamRead::Event {
        id: 1,
        event: FeedEvent::Reset { after: 1 },
    };
    let script = vec![stream(vec![revision(1), reset]), stream(vec![revision(2)])];
    let run = run(script, FAST, 2, OPEN);
    assert_eq!(run.result, Ok(()));
    assert!(run.told.contains(&FeedEvent::Reset { after: 1 }));
    assert_eq!(run.asked, vec![Some(0), Some(1)]);
}

#[test]
fn a_malformed_stream_or_a_refusal_ends_the_watch_at_once() {
    let malformed = stream(vec![StreamRead::Malformed("malformed: x".into())]);
    let run1 = run(vec![malformed, stream(vec![revision(1)])], FAST, 1, OPEN);
    assert_eq!(run1.result, Err("malformed: x".into()));
    assert_eq!(run1.asked.len(), 1);
    let run2 = run(vec![Open::Refused], FAST, 1, OPEN);
    assert_eq!(run2.result, Err("refused (503 unavailable)".into()));
    assert_eq!((run2.asked.len(), run2.paused.len()), (1, 0));
}

/// Opens that fail, or end with no event, end the watch after
/// `attempts` in a row; a stream with an event starts the count again.
#[test]
fn opens_failing_in_a_row_are_bounded() {
    let script = vec![
        Open::Unreachable,
        Open::Unreachable,
        Open::Unreachable,
        Open::Unreachable,
        stream(vec![revision(1)]),
        stream(vec![]),
    ];
    let run = run(script, FAST, 99, OPEN);
    let why = run.result.expect_err("bounded");
    assert_eq!(
        why,
        "the board server's stream ended with no event (5 times in a row)"
    );
    assert_eq!(run.asked.len(), 10, "4 failed, 1 heard, then 5 empty");
}

/// The stream's `retry:` is the next reopen's pause.
#[test]
fn a_streams_retry_is_its_reopen_pause() {
    let script = vec![
        Open::Stream(vec![revision(1)], Some(ms(700))),
        stream(vec![revision(2)]),
    ];
    let run = run(script, FAST, 2, OPEN);
    assert_eq!(run.result, Ok(()));
    assert_eq!(run.paused, vec![ms(700)]);
}

/// Review (#2757 round 3, L1): once the output has no reader, no stream is
/// opened again: before the first open, or between streams.
#[test]
fn a_gone_output_ends_the_watch_before_it_opens_again() {
    let none = run(vec![stream(vec![revision(1)])], FAST, 9, 0);
    assert_eq!((none.result, none.asked.len()), (Ok(()), 0));
    let script = vec![stream(vec![revision(1)]), stream(vec![revision(2)])];
    let one = run(script, FAST, 9, 1);
    assert_eq!(one.result, Ok(()));
    assert_eq!(one.asked, vec![Some(0)], "not opened again");
    assert!(one.paused.is_empty(), "no pause either");
}

/// Review (#2757 round 3, M1 is the adapter's; here): a busy server is
/// tried again until a place frees; a heard stream starts busy afresh.
#[test]
fn a_busy_server_is_tried_again() {
    let script = vec![Open::Busy, Open::Busy, stream(vec![revision(1)])];
    let run = run(script, FAST, 1, OPEN);
    assert_eq!(run.result, Ok(()));
    assert_eq!(run.asked.len(), 3);
}

/// Review (#2757 round 3, L2a): a reopen pause of 0 never makes busy spin:
/// every busy pause is at least the floor (whatever a reader's clamp does).
#[test]
fn busy_pauses_are_never_under_the_floor() {
    let timing = WatchTiming {
        busy_tries: 6,
        ..FAST
    };
    let run = run(vec![Open::Busy], timing, 1, OPEN);
    assert!(run.result.is_err());
    assert_eq!(run.paused.len(), 6);
    assert!(
        run.paused.iter().all(|paused| *paused >= ms(100)),
        "{:?}",
        run.paused
    );
}

/// Review (#2757 round 3, L2b): busy pauses double, from the floor to the
/// ceiling.
#[test]
fn busy_pauses_double_to_their_ceiling() {
    let timing = WatchTiming {
        max_backoff: ms(1_000),
        busy_tries: 6,
        ..FAST
    };
    let run = run(vec![Open::Busy], timing, 1, OPEN);
    assert_eq!(
        run.paused,
        vec![ms(100), ms(200), ms(400), ms(800), ms(1_000), ms(1_000)]
    );
}

/// Review (#2757 round 3, L2c): however long busy may last, a server is
/// tried at most `busy_tries` times more: `busy_tries + 1` opens.
#[test]
fn busy_tries_are_capped_at_their_count() {
    let timing = WatchTiming {
        busy_for: Duration::from_secs(3_600),
        busy_tries: 3,
        ..FAST
    };
    let run = run(vec![Open::Busy], timing, 1, OPEN);
    let why = run.result.expect_err("capped");
    assert!(why.starts_with("busy (still busy after"), "{why}");
    assert_eq!(run.asked.len(), 4);
}

/// Review (#2757 round 3, nit): the last busy pause is what is left of the
/// bound, so the watch never waits past it.
#[test]
fn busy_never_waits_past_its_bound() {
    let timing = WatchTiming {
        busy_for: ms(250),
        ..FAST
    };
    let run = run(vec![Open::Busy], timing, 1, OPEN);
    assert_eq!(run.paused, vec![ms(100), ms(150)]);
    let why = run.result.expect_err("bounded");
    assert!(why.starts_with("busy (still busy after"), "{why}");
    let total: Duration = run.paused.iter().sum();
    assert_eq!(total, ms(250));
}
