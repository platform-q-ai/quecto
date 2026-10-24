//! The board service's route table (epic #2705 B4), a fixed allowlist:
//! `GET /v1/status` `{role, board, head}`, `GET /v1/time` `{time}`, and
//! `GET /v1/boards/{id}/snapshot` `{board, head, files}` (stored bytes).
//! A path outside the table is 404; one asked by another method is 405.
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::{Value, json};

use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::{BoardStoreError, ServeError, ServedSnapshot};
use crate::application::project_board::use_cases::ServeBoard;
use crate::domain::project_board::value_objects::telemetry::BoardId;
use crate::domain::shared::redaction::{redact_secrets, redact_url_userinfo};
use crate::infrastructure::http::board_service::listener::{
    ServiceHandler, ServiceMethod, ServiceReply, ServiceRequest,
};

/// The routes, over the server's reads of its board.
pub struct BoardRoutes {
    serve: ServeBoard,
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

    /// The one method it takes.
    fn method(&self) -> ServiceMethod {
        match self {
            Self::Status | Self::Time | Self::Snapshot(_) => ServiceMethod::Get,
        }
    }
}

impl BoardRoutes {
    pub fn new(serve: ServeBoard) -> Self {
        Self { serve }
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
                Some(board) => match self.serve.snapshot(&board) {
                    Ok(snapshot) => snapshot_reply(snapshot),
                    Err(error) => failed(&error),
                },
                None => failed(&ServeError::UnknownBoard),
            },
        }
    }
}

impl ServiceHandler for BoardRoutes {
    fn answer(&self, request: &ServiceRequest) -> ServiceReply {
        match Route::of(&request.path) {
            Some(route) if route.method() == request.method => self.route(route),
            Some(_) => ServiceReply::json(405, &json!({ "error": "method not allowed" })),
            None => ServiceReply::json(404, &json!({ "error": "no such route" })),
        }
    }
}

/// A snapshot's reply: each stored file whole. A file of UTF-8 text with
/// no control character but tab and newline is sent as that text
/// (`"encoding": "utf-8"`), so a JSON string holds it in at most twice its
/// bytes; any other as base64; an entry set aside with no bytes as none.
fn snapshot_reply(snapshot: ServedSnapshot) -> ServiceReply {
    let files: Vec<Value> = snapshot
        .files
        .into_iter()
        .map(|file| {
            let (encoding, content) = match file.bytes {
                Some(bytes) => match String::from_utf8(bytes) {
                    Ok(text) if text.chars().all(plain) => ("utf-8", Value::String(text)),
                    Ok(text) => ("base64", Value::String(BASE64.encode(text.as_bytes()))),
                    Err(error) => ("base64", Value::String(BASE64.encode(error.as_bytes()))),
                },
                None => ("none", Value::Null),
            };
            json!({
                "path": file.path,
                "encoding": encoding,
                "content": content,
                "set_aside": file.set_aside,
            })
        })
        .collect();
    ServiceReply::json(
        200,
        &json!({
            "board": snapshot.board.as_str(),
            "head": snapshot.head.0,
            "files": files,
        }),
    )
}

/// A character a JSON string holds as itself, or as two bytes (`\n`,
/// `\t`, `\"`, `\\`).
fn plain(c: char) -> bool {
    !c.is_control() || c == '\n' || c == '\t'
}

/// What a failed read is answered with: 404 for no such board, 500 (and a
/// log line) for files that cannot be given whole and true or past one
/// answer's bound, and 503 while the store or clock cannot answer.
fn failed(error: &ServeError) -> ServiceReply {
    let (status, why) = match error {
        ServeError::UnknownBoard => (404, "no such board on this server".to_string()),
        ServeError::Board(error @ ProjectBoardError::Store(BoardStoreError::NoBoard)) => {
            (404, error.to_string())
        }
        ServeError::Board(error) => (503, error.to_string()),
        ServeError::Files(why) | ServeError::TooLarge(why) => (500, why.clone()),
    };
    // A store error may quote its remote: secrets and userinfo never leave.
    let why = redact_secrets(&redact_url_userinfo(&why));
    if status == 500 {
        tracing::error!(error = %why, "board server: a snapshot could not be served");
    }
    ServiceReply::json(status, &json!({ "error": why }))
}

#[cfg(test)]
#[path = "routes_tests.rs"]
pub(super) mod tests;
