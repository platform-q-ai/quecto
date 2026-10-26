//! The [`BoardWatch`] port over a board server (epic #2705 B10): its
//! `events` stream read, each event decoded by the feed's allowlisted
//! schema, and the stream opened again from the last id it sent whenever
//! it ends or is cut, so a watcher sees each revision once, with no gap.
//! A malformed stream ends the watch; a busy server is tried again for a
//! connection's lifetime and more; an epoch already handed on is not
//! handed on again.
use std::collections::BTreeSet;
use std::time::Duration;

use serde_json::Value;

use super::board_service::client::{BoardServiceClient, ClientError};
use super::board_service::events::{EventReader, Opened, StreamEvent};
use crate::application::project_board::ports::feed::{
    BoardRevision, BoardWatch, FeedEvent, WatchFlow,
};
use crate::domain::project_board::value_objects::telemetry::BoardId;

/// How long a read may wait: three of the server's 15 s keep-alives missed.
pub const IDLE: Duration = Duration::from_secs(45);
/// The pause before a stream is opened again, until a stream's `retry:`
/// (the server sends 1 s) says otherwise.
pub const PAUSE: Duration = Duration::from_secs(1);
/// Opens in a row that may fail, or end with no event, before the watch
/// gives up.
pub const ATTEMPTS: u32 = 5;
/// How long a busy server is tried again: past a connection's 60 s
/// lifetime, by which every stream holding a place has given it back.
pub const BUSY_FOR: Duration = Duration::from_secs(75);
/// The longest pause between tries of a busy server.
pub const MAX_BACKOFF: Duration = Duration::from_secs(8);
/// The most tries of a busy server, however short its pauses.
pub const MAX_BUSY_TRIES: u32 = 0;

/// A watch's times: how long a read may wait, the pause before a stream is
/// opened again (until a stream's `retry:` says otherwise), and how long a
/// busy server is tried again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchTiming {
    pub idle: Duration,
    pub pause: Duration,
    pub busy_for: Duration,
}

impl WatchTiming {
    pub const PRODUCTION: Self = Self {
        idle: IDLE,
        pause: PAUSE,
        busy_for: BUSY_FOR,
    };
}

pub struct RemoteBoardWatch {
    client: BoardServiceClient,
    board: BoardId,
    timing: WatchTiming,
}

/// Where a watch stands across its streams: the cursor it reopens from,
/// and the epoch it last handed on.
struct Standing {
    cursor: Option<u64>,
    epoch: Option<u64>,
}

/// How one stream ended: whether `each` stopped it, whether it gave any
/// event, and the reconnection delay it set.
struct Ended {
    flow: WatchFlow,
    heard: bool,
    retry: Option<Duration>,
}

impl RemoteBoardWatch {
    pub fn new(client: BoardServiceClient, board: BoardId) -> Self {
        Self::timed(client, board, WatchTiming::PRODUCTION)
    }

    /// A watch held to `timing`.
    pub fn timed(client: BoardServiceClient, board: BoardId, timing: WatchTiming) -> Self {
        Self {
            client,
            board,
            timing,
        }
    }

    /// Reads one stream to its end. An epoch the watch last handed on is
    /// not handed on again. A stream cut in transport ends quietly, to be
    /// opened again; one malformed, or an event off the schema, ends the
    /// watch.
    fn read(
        &self,
        mut reader: EventReader,
        standing: &mut Standing,
        each: &mut dyn FnMut(FeedEvent) -> WatchFlow,
    ) -> Result<Ended, String> {
        let mut heard = false;
        loop {
            let ended = |flow, heard, reader: &EventReader| Ended {
                flow,
                heard,
                retry: reader.retry(),
            };
            let event = match reader.next_event() {
                Ok(Some(event)) => event,
                Ok(None) => return Ok(ended(WatchFlow::Continue, heard, &reader)),
                Err(error) if error.transport() || matches!(error, ClientError::NoReply) => {
                    return Ok(ended(WatchFlow::Continue, heard, &reader));
                }
                Err(error) => return Err(format!("{}: {error}", self.client.endpoint())),
            };
            let (id, event) = decoded(&event).ok_or_else(|| {
                "the board server sent an event this client does not read".to_string()
            })?;
            (standing.cursor, heard) = (Some(id), true);
            match event {
                FeedEvent::Epoch { epoch } if standing.epoch == Some(epoch) => continue,
                FeedEvent::Epoch { epoch } => standing.epoch = Some(epoch),
                FeedEvent::Revision(_) | FeedEvent::Reset { .. } => {}
            }
            if each(event) == WatchFlow::Stop {
                return Ok(ended(WatchFlow::Stop, heard, &reader));
            }
        }
    }
}

impl BoardWatch for RemoteBoardWatch {
    /// Opens the stream again whenever it ends: after the stream's `retry:`
    /// (else the pause). Opens that fail, or end with no event,
    /// [`ATTEMPTS`] in a row end the watch; a busy server is tried again
    /// with a doubling backoff, apart, for its `busy_for`.
    fn watch(
        &self,
        after: Option<u64>,
        each: &mut dyn FnMut(FeedEvent) -> WatchFlow,
    ) -> Result<(), String> {
        let path = format!("/v1/boards/{}/events", self.board.as_str());
        let timing = self.timing;
        let mut standing = Standing {
            cursor: after,
            epoch: None,
        };
        let (mut failed, mut pause) = (0, timing.pause);
        // While busy: the pauses slept so far, and the next one. Counted, so
        // the client reads no clock.
        let mut busy: Option<(Duration, Duration)> = None;
        loop {
            let failure = match self.client.open_events(&path, standing.cursor, timing.idle) {
                // Busy (a token's 2 streams, or the server's 32): a place
                // frees within a connection's lifetime.
                Ok(Opened::Refused(reply)) if reply.status == 503 => {
                    let (waited, backoff) = *busy.get_or_insert((Duration::ZERO, pause));
                    if waited >= timing.busy_for {
                        let why = refusal(reply.status, &reply.body);
                        let waited = waited.as_secs_f64();
                        return Err(format!("{why} (still busy after {waited:.1} s)"));
                    }
                    std::thread::sleep(backoff);
                    busy = Some((waited + backoff, (backoff * 2).min(MAX_BACKOFF)));
                    continue;
                }
                Ok(Opened::Refused(reply)) => return Err(refusal(reply.status, &reply.body)),
                Ok(Opened::Stream(reader)) => {
                    let ended = self.read(reader, &mut standing, each)?;
                    if ended.flow == WatchFlow::Stop {
                        return Ok(());
                    }
                    pause = ended.retry.unwrap_or(pause);
                    match ended.heard {
                        true => {
                            (failed, busy) = (0, None);
                            None
                        }
                        false => Some(format!(
                            "{}: the board server's stream ended with no event",
                            self.client.endpoint()
                        )),
                    }
                }
                Err(error) if error.transport() || matches!(error, ClientError::NoReply) => {
                    Some(format!("{}: {error}", self.client.endpoint()))
                }
                Err(error) => return Err(format!("{}: {error}", self.client.endpoint())),
            };
            if let Some(why) = failure {
                failed += 1;
                if failed >= ATTEMPTS {
                    return Err(format!("{why} ({failed} times in a row)"));
                }
            }
            std::thread::sleep(pause);
        }
    }
}

/// An event by the feed's schema, an allowlist: its id and what it tells;
/// `None` for anything else (another kind, another key, an id that is not
/// the event's cursor).
pub fn decoded(event: &StreamEvent) -> Option<(u64, FeedEvent)> {
    let id = event.id.as_deref().and_then(number)?;
    let data: Value = serde_json::from_str(&event.data).ok()?;
    let fields = data.as_object()?;
    let keys: BTreeSet<&str> = fields.keys().map(String::as_str).collect();
    let field = |key: &str| fields.get(key).and_then(Value::as_u64);
    let shaped = |want: &[&str]| keys == want.iter().copied().collect();
    match (
        event.event.as_str(),
        fields.get("kind").and_then(Value::as_str),
    ) {
        ("epoch", Some("epoch")) if shaped(&["kind", "epoch"]) => Some((
            id,
            FeedEvent::Epoch {
                epoch: field("epoch")?,
            },
        )),
        ("revision", Some("revision")) if shaped(&["kind", "epoch", "revision"]) => {
            let revision = field("revision").filter(|revision| *revision == id)?;
            let epoch = field("epoch")?;
            Some((id, FeedEvent::Revision(BoardRevision { epoch, revision })))
        }
        ("reset", Some("reset")) if shaped(&["kind", "after"]) => {
            let after = field("after").filter(|after| *after == id)?;
            Some((id, FeedEvent::Reset { after }))
        }
        (_, _) => None,
    }
}

fn number(text: &str) -> Option<u64> {
    let digits = (1..=20).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit());
    digits.then(|| text.parse().ok()).flatten()
}

/// A refused open, by the server's status and code.
fn refusal(status: u16, body: &[u8]) -> String {
    let reply: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let shown = |text: &str| {
        text.chars()
            .filter(|c| !c.is_control())
            .take(512)
            .collect::<String>()
    };
    let code = shown(reply["code"].as_str().unwrap_or("none"));
    let why = shown(reply["error"].as_str().unwrap_or("no reason given"));
    format!("the board server refused the watch ({status} {code}): {why}")
}

#[cfg(test)]
#[path = "board_watch_tests.rs"]
mod tests;
