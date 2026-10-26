//! The [`BoardWatch`] port over a board server (epic #2705 B10): its
//! `events` stream read, each event decoded by the feed's allowlisted
//! schema, and the stream opened again from the last id it sent whenever
//! it ends or is cut, so a watcher sees each revision once, with no gap.
use std::collections::BTreeSet;
use std::time::Duration;

use serde_json::Value;

use super::board_service::client::{BoardServiceClient, ClientError};
use super::board_service::events::{Opened, StreamEvent};
use crate::application::project_board::ports::feed::{
    BoardRevision, BoardWatch, FeedEvent, WatchFlow,
};
use crate::domain::project_board::value_objects::telemetry::BoardId;

/// How long a read may wait: past two of the server's 15 s keep-alives.
pub const IDLE: Duration = Duration::from_secs(45);
/// The pause before a stream is opened again (the server's `retry:`).
pub const PAUSE: Duration = Duration::from_secs(1);
/// Opens that may fail in a row before the watch gives up.
pub const ATTEMPTS: u32 = 5;

/// How long a busy server is tried again: past a connection's 60 s
/// lifetime, by which every stream holding a place has given it back.
pub const BUSY_FOR: Duration = Duration::from_secs(75);

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
    idle: Duration,
    pause: Duration,
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
            idle: timing.idle,
            pause: timing.pause,
        }
    }

    /// Reads one opened stream to its end: whether `each` stopped it.
    fn read(
        &self,
        opened: Opened,
        cursor: &mut Option<u64>,
        each: &mut dyn FnMut(FeedEvent) -> WatchFlow,
    ) -> Result<WatchFlow, String> {
        let mut reader = match opened {
            Opened::Stream(reader) => reader,
            Opened::Refused(reply) => return Err(refusal(reply.status, &reply.body)),
        };
        loop {
            let event = match reader.next_event() {
                Ok(Some(event)) => event,
                // Ended or cut: opened again from the cursor.
                Ok(None) | Err(_) => return Ok(WatchFlow::Continue),
            };
            let (id, event) = decoded(&event).ok_or_else(|| {
                "the board server sent an event this client does not read".to_string()
            })?;
            *cursor = Some(id);
            if each(event) == WatchFlow::Stop {
                return Ok(WatchFlow::Stop);
            }
        }
    }
}

impl BoardWatch for RemoteBoardWatch {
    fn watch(
        &self,
        after: Option<u64>,
        each: &mut dyn FnMut(FeedEvent) -> WatchFlow,
    ) -> Result<(), String> {
        let path = format!("/v1/boards/{}/events", self.board.as_str());
        let (mut cursor, mut failed) = (after, 0);
        loop {
            let failure = match self.client.open_events(&path, cursor, self.idle) {
                // Busy (a token's 2 streams, or the server's 32): tried again.
                Ok(Opened::Refused(reply)) if reply.status == 503 => {
                    Some(refusal(reply.status, &reply.body))
                }
                Ok(opened) => {
                    failed = 0;
                    match self.read(opened, &mut cursor, each)? {
                        WatchFlow::Stop => return Ok(()),
                        WatchFlow::Continue => None,
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
                    return Err(why);
                }
            }
            std::thread::sleep(self.pause);
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
