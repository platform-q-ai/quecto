//! The board service's route table (epic #2705 B4, B5, B7), an allowlist:
//! reads `GET /v1/status` and `GET /v1/time`; the board's operations
//! `POST /v1/boards/{id}/op` ([`super::op`]); admin `GET`/`POST
//! /v1/admin/tokens` and `POST /v1/admin/tokens/revoke`, on the Unix socket
//! only (403 over TCP); else 404, or 405 naming a route's methods.
use std::sync::PoisonError;

use serde_json::json;

use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::{
    BoardClockError, BoardStoreError, ServeError, TokenError,
};
use crate::application::project_board::use_cases::{
    BoardAccess, Principal, Refused, Scope, ServeBoard, ServeBoardOperation,
};
use crate::domain::project_board::value_objects::telemetry::BoardId;
use crate::domain::shared::redaction::{redact_secrets, redact_url_userinfo};
use crate::infrastructure::http::board_service::listener::{
    ServiceHandler, ServiceMethod, ServiceReply, ServiceRequest, Transport,
};

use super::op::{OpRefusal, answer_op};

/// The routes, over the server's reads, operations and access rules.
pub struct BoardRoutes {
    serve: ServeBoard,
    access: BoardAccess,
    operations: ServeBoardOperation,
    /// The token store's failure last said on stderr.
    unavailable: std::sync::Mutex<String>,
}

/// A route of the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    Status,
    Time,
    Tokens,
    Revoke,
    Op,
}

impl Route {
    /// The route `path` names, if the table holds it.
    fn of(path: &str) -> Option<Self> {
        match path {
            "/v1/status" => Some(Self::Status),
            "/v1/time" => Some(Self::Time),
            "/v1/admin/tokens" => Some(Self::Tokens),
            "/v1/admin/tokens/revoke" => Some(Self::Revoke),
            other => board_of(other).map(|_| Self::Op),
        }
    }

    /// The methods it takes, as an `Allow` value.
    fn allow(self) -> &'static str {
        match self {
            Self::Status | Self::Time => "GET",
            Self::Tokens => "GET, POST",
            Self::Revoke | Self::Op => "POST",
        }
    }

    fn takes(self, method: ServiceMethod) -> bool {
        self.allow()
            .split(", ")
            .any(|allowed| allowed == method.as_str())
    }

    /// The scope a caller needs to reach it (an op's own is judged after).
    fn scope(self) -> Scope {
        match self {
            Self::Status | Self::Time | Self::Op => Scope::Read,
            Self::Tokens | Self::Revoke => Scope::Admin,
        }
    }
}

/// The board id of an op route's `path`, when it is shaped as one.
fn board_of(path: &str) -> Option<BoardId> {
    path.strip_prefix("/v1/boards/")
        .and_then(|rest| rest.strip_suffix("/op"))
        .and_then(BoardId::of_hex)
}

/// An admin request's body: the token's name.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Named {
    name: String,
}

impl BoardRoutes {
    pub fn new(serve: ServeBoard, access: BoardAccess, operations: ServeBoardOperation) -> Self {
        Self {
            serve,
            access,
            operations,
            unavailable: std::sync::Mutex::default(),
        }
    }

    /// An op's reply, or its refusal by its code.
    fn op(&self, request: &ServiceRequest, principal: &Principal) -> ServiceReply {
        match board_of(&request.path) {
            Some(board) if board == *self.serve.board() => {}
            Some(_) | None => return coded(404, "board", "this server serves no such board"),
        }
        match answer_op(&self.operations, principal, &request.body) {
            Ok(answer) => ServiceReply {
                body: answer.into_bytes(),
                ..ServiceReply::json(200, &serde_json::Value::Null)
            },
            Err(OpRefusal::Body(why)) => coded(400, "request", &why),
            Err(OpRefusal::Access(refused)) => self.refusal(&refused, request.transport),
            Err(OpRefusal::Serve(error)) => failed(&error),
        }
    }

    /// A refused request's reply: 401 `token`, 403 `local_only`, or 503;
    /// why the tokens failed is said on the Unix socket only.
    fn refusal(&self, refused: &Refused, transport: Transport) -> ServiceReply {
        match refused {
            Refused::Token => ServiceReply {
                challenge: Some("Bearer"),
                ..coded(
                    401,
                    "token",
                    "unauthorized: a valid bearer token is required",
                )
            },
            Refused::LocalOnly => coded(
                403,
                "local_only",
                "forbidden: admin routes and operations are served on the Unix socket only",
            ),
            Refused::Unavailable(error) => match transport {
                Transport::Local => coded(503, "unavailable", &error.to_string()),
                Transport::Tcp => {
                    self.log_unavailable(error);
                    coded(503, "unavailable", "the token store is unavailable")
                }
            },
        }
    }

    /// The token store's failure on stderr, only when it changes.
    fn log_unavailable(&self, error: &TokenError) {
        let mut last = self
            .unavailable
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if changed(&mut last, &error.to_string()) {
            eprintln!("board-server: the token store is unavailable: {error}");
        }
    }

    /// Answers `request` from `principal` on `route`, a method it takes.
    fn route(&self, route: Route, request: &ServiceRequest, principal: &Principal) -> ServiceReply {
        match (route, request.method) {
            (Route::Op, _) => self.op(request, principal),
            (Route::Status, _) => read(self.serve.status().map(|status| {
                json!({
                    "role": status.role.as_str(),
                    "board": status.board.as_str(),
                    "head": status.head.map(|head| head.0),
                })
            })),
            (Route::Time, _) => read(
                self.serve
                    .time()
                    .map(|time| json!({ "time": time.as_str() })),
            ),
            (Route::Tokens, ServiceMethod::Get) => administer(self.access.list().map(|records| {
                let tokens: Vec<_> = records
                    .iter()
                    .map(|token| json!({"name": token.name.as_str(), "created": token.created.as_str()}))
                    .collect();
                (200, json!({ "tokens": tokens }))
            })),
            (Route::Tokens, ServiceMethod::Post) => named(&request.body, |name| {
                administer(self.access.add(name).map(|issued| {
                    let record = &issued.record;
                    let body = json!({"name": record.name.as_str(),
                        "created": record.created.as_str(), "token": issued.secret});
                    (201, body)
                }))
            }),
            (Route::Revoke, _) => named(&request.body, |name| {
                administer(self.access.revoke(name).map(|name| {
                    (200, json!({ "name": name.as_str(), "revoked": true }))
                }))
            }),
        }
    }
}

/// A read's reply.
fn read(answered: Result<serde_json::Value, ServeError>) -> ServiceReply {
    match answered {
        Ok(body) => ServiceReply::json(200, &body),
        Err(error) => failed(&error),
    }
}

/// An admin command's reply.
fn administer(answered: Result<(u16, serde_json::Value), TokenError>) -> ServiceReply {
    match answered {
        Ok((status, body)) => ServiceReply::json(status, &body),
        Err(error) => token_failed(&error),
    }
}

impl ServiceHandler for BoardRoutes {
    /// A TCP caller with no held token is 401; a held token's name is the
    /// principal the listener hands to `answer`.
    fn authenticate_remote(&self, authorization: Option<&str>) -> Result<String, ServiceReply> {
        let device = self.access.authenticate(authorization.and_then(bearer));
        let device = device.map_err(|refused| self.refusal(&refused, Transport::Tcp))?;
        Ok(device.as_str().to_string())
    }

    fn answer(&self, request: &ServiceRequest) -> ServiceReply {
        let principal = match (request.transport, request.principal.as_deref()) {
            (Transport::Local, _) => Ok(Principal::Local),
            (Transport::Tcp, Some(name)) => BoardAccess::device(name),
            (Transport::Tcp, None) => Err(Refused::Token),
        };
        let principal = match principal {
            Ok(principal) => principal,
            Err(refused) => return self.refusal(&refused, request.transport),
        };
        match Route::of(&request.path) {
            Some(route) if route.takes(request.method) => {
                match BoardAccess::authorize(&principal, route.scope()) {
                    Ok(()) => self.route(route, request, &principal),
                    Err(refused) => self.refusal(&refused, request.transport),
                }
            }
            Some(route) => ServiceReply {
                allow: Some(route.allow()),
                ..coded(405, "method", "method not allowed")
            },
            None => coded(404, "route", "no such route"),
        }
    }

    fn allowed(&self, path: &str) -> Option<&'static str> {
        Route::of(path).map(Route::allow)
    }
}

/// The token of `Bearer <token>` (the scheme in any case), else `None`.
fn bearer(value: &str) -> Option<&str> {
    let (scheme, token) = value.split_once(' ')?;
    let token_shaped = !token.is_empty() && token.bytes().all(|byte| byte.is_ascii_graphic());
    match scheme.eq_ignore_ascii_case("bearer") && token_shaped {
        true => Some(token),
        false => None,
    }
}

/// `answer` of an admin body's name, or a 400 `request`.
fn named(body: &[u8], answer: impl FnOnce(&str) -> ServiceReply) -> ServiceReply {
    match serde_json::from_slice::<Named>(body) {
        Ok(named) => answer(&named.name),
        Err(_) => coded(
            400,
            "request",
            "the body must be {\"name\": \"<token name>\"}",
        ),
    }
}

/// Whether `now` differs from `last`, which then becomes it.
pub(super) fn changed(last: &mut String, now: &str) -> bool {
    match last.as_str() == now {
        true => false,
        false => {
            now.clone_into(last);
            true
        }
    }
}

/// An error reply with its typed `code`.
fn coded(status: u16, code: &str, why: &str) -> ServiceReply {
    ServiceReply::json(status, &json!({ "error": why, "code": code }))
}

/// A failed token command's reply, each error its status and code.
fn token_failed(error: &TokenError) -> ServiceReply {
    let (status, code) = match error {
        TokenError::BadName(_) => (400, "token_name"),
        TokenError::Exists(_) => (409, "token_exists"),
        TokenError::Full => (409, "token_limit"),
        TokenError::NoSuchToken(_) => (404, "no_token"),
        TokenError::Unavailable(_) => (503, "unavailable"),
    };
    coded(status, code, &error.to_string())
}

/// A failed request's reply: 404 `no_board` while there is no board, 400
/// `write` for a write without its id and committer, 503 `clock_unsynced`
/// with the host's `reason` while its clock is not synchronised (#2705),
/// else 503 `unavailable`; a store error may quote its remote, so it is
/// redacted.
fn failed(error: &ServeError) -> ServiceReply {
    let shown = |text: &str| redact_secrets(&redact_url_userinfo(text));
    if let ServeError::Board(clock @ ProjectBoardError::Clock(BoardClockError::Unsynced(why))) =
        error
    {
        let (message, reason) = (shown(&clock.to_string()), shown(why));
        let body = json!({ "error": message, "code": "clock_unsynced", "reason": reason });
        return ServiceReply::json(503, &body);
    }
    let (status, code, why) = match error {
        ServeError::Board(error @ ProjectBoardError::Store(BoardStoreError::NoBoard)) => {
            (404, "no_board", error.to_string())
        }
        ServeError::Board(error) => (503, "unavailable", error.to_string()),
        ServeError::NoWrite => (
            400,
            "write",
            "a write needs its write_id and committer".to_string(),
        ),
        ServeError::WriteIdReused => (
            409,
            "write_id_reused",
            "this write_id was used for another write: make a new one".to_string(),
        ),
        ServeError::Busy => (
            503,
            "busy",
            "too many writes at once: try again shortly".into(),
        ),
    };
    coded(status, code, &shown(&why))
}

#[cfg(test)]
#[path = "routes_tests.rs"]
pub(super) mod tests;
