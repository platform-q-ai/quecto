//! The feed's streams over a board server (epic #2705 B10): its `events`
//! stream opened from a cursor ([`BoardStreams`]), each event decoded by
//! the feed's allowlisted schema ([`BoardStream`]). Which refusal is busy
//! is an allowlist: a 503 whose code is `busy` alone; every other refusal
//! ends the watch. A stream cut in transport ends quietly, to be opened
//! again; a malformed one is said. The watch's policy is the application's
//! (`WatchBoard`); this adapter only opens and reads.
use std::collections::BTreeSet;
use std::os::fd::RawFd;
use std::time::Duration;

use serde_json::Value;

use super::board_service::client::{BoardServiceClient, ClientError};
use super::board_service::events::{EventReader, Opened, StreamEvent};
use crate::application::project_board::ports::feed::{
    BoardRevision, BoardStream, BoardStreams, FeedEvent, OpenRefused, StreamRead, Told, WatchPauses,
};
use crate::domain::project_board::value_objects::telemetry::BoardId;

/// How long a read may wait: three of the server's 15 s keep-alives missed.
pub const IDLE: Duration = Duration::from_secs(45);

/// The feed of the board `board` on the server `client` reaches.
pub struct RemoteBoardStreams {
    client: BoardServiceClient,
    board: BoardId,
    idle: Duration,
}

impl RemoteBoardStreams {
    pub fn new(client: BoardServiceClient, board: BoardId) -> Self {
        Self::idling(client, board, IDLE)
    }

    /// Streams whose reads wait at most `idle`.
    pub fn idling(client: BoardServiceClient, board: BoardId, idle: Duration) -> Self {
        Self {
            client,
            board,
            idle,
        }
    }
}

impl BoardStreams for RemoteBoardStreams {
    fn open(&self, after: Option<u64>) -> Result<Box<dyn BoardStream>, OpenRefused> {
        let path = format!("/v1/boards/{}/events", self.board.as_str());
        let endpoint = self.client.endpoint().to_string();
        match self.client.open_events(&path, after, self.idle) {
            Ok(Opened::Stream(reader)) => Ok(Box::new(RemoteStream { reader, endpoint })),
            // Busy (a token's 2 streams, or the server's 32): a place frees
            // within a connection's lifetime. Only that 503 is tried again.
            Ok(Opened::Refused(reply)) if reply.status == 503 && code_of(&reply.body) == "busy" => {
                Err(OpenRefused::Busy(refusal(reply.status, &reply.body)))
            }
            Ok(Opened::Refused(reply)) => {
                Err(OpenRefused::Refused(refusal(reply.status, &reply.body)))
            }
            Err(error) if error.transport() || matches!(error, ClientError::NoReply) => {
                Err(OpenRefused::Unreachable(format!("{endpoint}: {error}")))
            }
            Err(error) => Err(OpenRefused::Refused(format!("{endpoint}: {error}"))),
        }
    }
}

/// One open stream.
struct RemoteStream {
    reader: EventReader,
    endpoint: String,
}

impl BoardStream for RemoteStream {
    fn read(&mut self) -> StreamRead {
        match self.reader.next_event() {
            Ok(Some(event)) => match decoded(&event) {
                Some((id, event)) => StreamRead::Event { id, event },
                None => StreamRead::Malformed(
                    "the board server sent an event this client does not read".into(),
                ),
            },
            Ok(None) => StreamRead::Ended,
            Err(error) if error.transport() || matches!(error, ClientError::NoReply) => {
                StreamRead::Ended
            }
            Err(error) => StreamRead::Malformed(format!("{}: {error}", self.endpoint)),
        }
    }

    fn retry(&self) -> Option<Duration> {
        self.reader.retry()
    }
}

/// A watch's pauses: the thread sleeps.
#[derive(Debug, Clone, Copy, Default)]
pub struct ThreadPauses;

impl WatchPauses for ThreadPauses {
    fn pause(&self, how_long: Duration) {
        std::thread::sleep(how_long);
    }
}

/// Whether standard output still has a reader: [`has_reader`] of fd 1.
pub fn stdout_has_reader() -> bool {
    has_reader(1)
}

/// Whether the descriptor `fd` still has a reader: a pipe whose reader has
/// gone polls `POLLERR`, a hung-up terminal `POLLHUP`; any other answer,
/// a failed poll among them, counts as a reader (a write then says).
pub fn has_reader(fd: RawFd) -> bool {
    let mut polled = libc::pollfd {
        fd,
        events: 0,
        revents: 0,
    };
    // A zero timeout: `poll` returns at once, writing only `revents`.
    // SAFETY: one valid `pollfd`, its count 1, borrowed for the call.
    let ready = unsafe { libc::poll(&mut polled, 1, 0) };
    let gone = libc::POLLERR | libc::POLLHUP | libc::POLLNVAL;
    match ready {
        1 => polled.revents & gone == 0,
        _ => true,
    }
}

/// A refusal's typed `code`, or `""`.
fn code_of(body: &[u8]) -> String {
    let reply: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    reply["code"].as_str().unwrap_or_default().to_string()
}

/// An event by the feed's schema, an allowlist: its id and what it tells;
/// `None` for anything else (another kind, another key, an id that is not
/// the event's cursor).
pub fn decoded(event: &StreamEvent) -> Option<(u64, FeedEvent)> {
    let id = event.id.as_deref().and_then(Told::parse_id)?;
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
