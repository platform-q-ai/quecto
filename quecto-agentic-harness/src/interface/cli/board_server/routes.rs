//! The board service's route table (epic #2705 B4), a fixed allowlist:
//! `GET /v1/status` `{role, board, head}`, `GET /v1/time` `{time}`, and
//! `GET /v1/boards/{id}/snapshot` `{board, head, files}` (stored bytes).
//! A path outside the table is 404; one asked by another method is 405.
//! One snapshot is served at a time (each holds its board's bytes): a
//! second waits [`SNAPSHOT_WAIT`], then is answered 503 `busy`.
use std::io::Write;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::json;

use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::{BoardStoreError, ServeError, ServedSnapshot};
use crate::application::project_board::use_cases::ServeBoard;
use crate::domain::project_board::value_objects::telemetry::BoardId;
use crate::domain::shared::redaction::{redact_secrets, redact_url_userinfo};
use crate::infrastructure::http::board_service::listener::{
    ServiceHandler, ServiceMethod, ServiceReply, ServiceRequest,
};

/// How long a snapshot waits for the one before it.
pub const SNAPSHOT_WAIT: Duration = Duration::from_secs(5);

/// The routes, over the server's reads of its board.
pub struct BoardRoutes {
    serve: ServeBoard,
    snapshots: OneAtATime,
    wait: Duration,
}

/// A route of the table.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Route {
    Status,
    Time,
    Snapshot(String),
}

impl Route {
    /// The route `path` names, if the table holds it.
    fn of(path: &str) -> Option<Self> {
        let parts: Vec<&str> = path.strip_prefix('/')?.split('/').collect();
        match parts.as_slice() {
            ["v1", "status"] => Some(Self::Status),
            ["v1", "time"] => Some(Self::Time),
            ["v1", "boards", id, "snapshot"] => Some(Self::Snapshot((*id).to_string())),
            _ => None,
        }
    }

    /// The one method it takes, and its `Allow` value.
    fn method(&self) -> (ServiceMethod, &'static str) {
        match self {
            Self::Status | Self::Time | Self::Snapshot(_) => (ServiceMethod::Get, "GET"),
        }
    }
}

/// A gate one holder passes at a time; the others wait a bounded time.
#[derive(Default)]
struct OneAtATime {
    busy: Mutex<bool>,
    freed: Condvar,
}

/// The gate, held until dropped.
struct Held<'a>(&'a OneAtATime);

impl OneAtATime {
    fn enter(&self, wait: Duration) -> Option<Held<'_>> {
        let busy = self.busy.lock().ok()?;
        let (mut busy, _) = self
            .freed
            .wait_timeout_while(busy, wait, |busy| *busy)
            .ok()?;
        match *busy {
            false => {
                *busy = true;
                Some(Held(self))
            }
            true => None,
        }
    }
}

impl Drop for Held<'_> {
    fn drop(&mut self) {
        if let Ok(mut busy) = self.0.busy.lock() {
            *busy = false;
        }
        self.0.freed.notify_one();
    }
}

impl BoardRoutes {
    pub fn new(serve: ServeBoard) -> Self {
        let (snapshots, wait) = (OneAtATime::default(), SNAPSHOT_WAIT);
        Self {
            serve,
            snapshots,
            wait,
        }
    }

    fn route(&self, route: Route) -> ServiceReply {
        match route {
            Route::Status => match self.serve.status() {
                Ok(status) => ServiceReply::json(
                    200,
                    &json!({
                        "role": status.role.as_str(),
                        "board": status.board.as_str(),
                        "head": status.head.map(|head| head.0),
                    }),
                ),
                Err(error) => failed(&error),
            },
            Route::Time => match self.serve.time() {
                Ok(time) => ServiceReply::json(200, &json!({ "time": time.as_str() })),
                Err(error) => failed(&error),
            },
            // An id that is not a board id names no board served.
            Route::Snapshot(id) => match BoardId::of_hex(&id) {
                Some(board) => match self.snapshots.enter(self.wait) {
                    Some(_held) => match self.serve.snapshot(&board) {
                        Ok(snapshot) => snapshot_reply(snapshot),
                        Err(error) => failed(&error),
                    },
                    None => coded(503, "busy", "busy: another snapshot is being served"),
                },
                None => failed(&ServeError::UnknownBoard),
            },
        }
    }
}

impl ServiceHandler for BoardRoutes {
    fn answer(&self, request: &ServiceRequest) -> ServiceReply {
        match Route::of(&request.path) {
            Some(route) if route.method().0 == request.method => self.route(route),
            Some(route) => ServiceReply {
                allow: Some(route.method().1),
                ..coded(405, "method", "method not allowed")
            },
            None => coded(404, "route", "no such route"),
        }
    }

    fn allowed(&self, path: &str) -> Option<&'static str> {
        Route::of(path).map(|route| route.method().1)
    }
}

/// A snapshot's reply, written straight into one buffer, each file's
/// bytes dropped once written. A file of UTF-8 text with no control
/// character but tab and newline is sent as that text (at most twice its
/// bytes in JSON); any other as base64; one with no bytes as none.
fn snapshot_reply(snapshot: ServedSnapshot) -> ServiceReply {
    let held: usize = snapshot
        .files
        .iter()
        .map(|file| file.bytes.as_ref().map_or(0, Vec::len))
        .sum();
    let mut body = Vec::with_capacity(held.saturating_add(held / 3).saturating_add(4096));
    match write_snapshot(&mut body, snapshot) {
        Ok(()) => ServiceReply {
            status: 200,
            body,
            allow: None,
        },
        Err(error) => failed(&ServeError::Files(error.to_string())),
    }
}

fn write_snapshot(out: &mut Vec<u8>, snapshot: ServedSnapshot) -> std::io::Result<()> {
    let text = |out: &mut Vec<u8>, text: &str| serde_json::to_writer(&mut *out, text);
    out.write_all(b"{\"board\":")?;
    text(out, snapshot.board.as_str())?;
    out.write_all(b",\"head\":")?;
    text(out, &snapshot.head.0)?;
    out.write_all(b",\"files\":[")?;
    for (n, file) in snapshot.files.into_iter().enumerate() {
        out.write_all(&b",{\"path\":"[usize::from(n == 0)..])?;
        text(out, &file.path)?;
        let binary = match file.bytes.map(String::from_utf8) {
            Some(Ok(plain)) if plain.chars().all(plain_char) => {
                out.write_all(b",\"encoding\":\"utf-8\",\"content\":")?;
                text(out, &plain)?;
                None
            }
            Some(Ok(other)) => Some(other.into_bytes()),
            Some(Err(error)) => Some(error.into_bytes()),
            None => {
                out.write_all(b",\"encoding\":\"none\",\"content\":null")?;
                None
            }
        };
        if let Some(bytes) = binary {
            out.write_all(b",\"encoding\":\"base64\",\"content\":\"")?;
            let mut encoder = base64::write::EncoderWriter::new(&mut *out, &BASE64);
            encoder.write_all(&bytes)?;
            encoder.finish()?.write_all(b"\"")?;
        }
        out.write_all(b",\"set_aside\":")?;
        serde_json::to_writer(&mut *out, &file.set_aside)?;
        out.write_all(b"}")?;
    }
    out.write_all(b"]}")
}

/// A character a JSON string holds as itself, or as two bytes (`\n`,
/// `\t`, `\"`, `\\`).
fn plain_char(c: char) -> bool {
    matches!(c, '\n' | '\t') || !c.is_control()
}

/// An error reply with its typed `code`.
fn coded(status: u16, code: &str, why: &str) -> ServiceReply {
    ServiceReply::json(status, &json!({ "error": why, "code": code }))
}

/// What a failed read is answered with: 404 for no such board, 413 for a
/// board past one answer's bound, 500 (and a log line) for files that
/// cannot be given whole and true, and 503 while the store or clock
/// cannot answer.
fn failed(error: &ServeError) -> ServiceReply {
    let (status, code, why) = match error {
        ServeError::UnknownBoard => (404, "board", "no such board on this server".to_string()),
        ServeError::Board(error @ ProjectBoardError::Store(BoardStoreError::NoBoard)) => {
            (404, "no_board", error.to_string())
        }
        ServeError::Board(error) => (503, "unavailable", error.to_string()),
        ServeError::TooLarge(why) => (413, "too_large", why.clone()),
        ServeError::Files(why) => (500, "files", why.clone()),
    };
    // A store error may quote its remote: secrets and userinfo never leave.
    let why = redact_secrets(&redact_url_userinfo(&why));
    if status == 500 {
        tracing::error!(error = %why, "board server: a snapshot could not be served");
    }
    coded(status, code, &why)
}

#[cfg(test)]
#[path = "routes_tests.rs"]
pub(super) mod tests;
