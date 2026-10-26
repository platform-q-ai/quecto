//! The board service's route table (epic #2705 B4, B5, B7), an allowlist:
//! reads `GET /v1/status` and `GET /v1/time`; the board's operations
//! `POST /v1/boards/{id}/op` ([`super::op`]); the change feed (B10), `GET
//! /v1/boards/{id}/events` and `GET /v1/boards/{id}/revisions`
//! ([`super::feed`]); a primary's batches for its followers (B11), `GET
//! /v1/boards/{id}/replicate`; admin `GET`/`POST /v1/admin/tokens` and
//! `POST /v1/admin/tokens/revoke`, on the Unix socket only (403 over TCP);
//! else 404, or 405 naming a route's methods. A follower (B11) answers
//! every write 409 `standby`, naming its primary.
use std::sync::PoisonError;

use serde_json::json;

use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::{
    BoardClockError, BoardStoreError, ServeError, TokenError, TokenKind,
};
use crate::application::project_board::use_cases::{
    BoardAccess, FollowBoard, Principal, Refused, Scope, ServeBoard, ServeBoardOperation,
    ServeReplication,
};
use crate::domain::project_board::value_objects::telemetry::BoardId;
use crate::domain::shared::redaction::{redact_secrets, redact_url_userinfo};
use crate::infrastructure::http::board_service::listener::{
    ServiceHandler, ServiceMethod, ServiceReply, ServiceRequest, ServiceStream, StreamWake,
    Transport,
};

use super::op::{OpRefusal, answer_op};

/// The routes, over the server's reads, operations and access rules.
pub struct BoardRoutes {
    serve: ServeBoard,
    access: BoardAccess,
    operations: ServeBoardOperation,
    /// The change feed (B10); none answers its routes 404.
    feed: Option<FollowBoard>,
    /// The batches for followers (B11); none answers the route 404.
    replication: Option<ServeReplication>,
    /// A follower's primary (B11): every write is refused, naming it.
    standby: Option<String>,
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
    Events,
    Revisions,
    Replicate,
}

/// The routes under a board's id, by their last segment.
const BOARD_ROUTES: [(&str, Route); 4] = [
    ("/op", Route::Op),
    ("/events", Route::Events),
    ("/revisions", Route::Revisions),
    ("/replicate", Route::Replicate),
];

impl Route {
    /// The route `path` names, if the table holds it.
    fn of(path: &str) -> Option<Self> {
        match path {
            "/v1/status" => Some(Self::Status),
            "/v1/time" => Some(Self::Time),
            "/v1/admin/tokens" => Some(Self::Tokens),
            "/v1/admin/tokens/revoke" => Some(Self::Revoke),
            other => board_of(other).map(|(_, route)| route),
        }
    }

    /// The methods it takes, as an `Allow` value.
    fn allow(self) -> &'static str {
        match self {
            Self::Status | Self::Time | Self::Events | Self::Revisions | Self::Replicate => "GET",
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
            Self::Status | Self::Time | Self::Op | Self::Events | Self::Revisions => Scope::Read,
            Self::Replicate => Scope::Replicate,
            Self::Tokens | Self::Revoke => Scope::Admin,
        }
    }
}

/// The board id and route of a board route's `path`, when it is shaped as
/// one: `/v1/boards/<16 hex>` and one of [`BOARD_ROUTES`].
fn board_of(path: &str) -> Option<(BoardId, Route)> {
    let rest = path.strip_prefix("/v1/boards/")?;
    BOARD_ROUTES.iter().find_map(|(last, route)| {
        let board = rest.strip_suffix(last).and_then(BoardId::of_hex)?;
        Some((board, *route))
    })
}

/// An admin request's body: the token's name.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Named {
    name: String,
}

/// A token's making: its name, and whether it is a follower's (B11).
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Adding {
    name: String,
    #[serde(default)]
    replica: bool,
}

impl BoardRoutes {
    pub fn new(serve: ServeBoard, access: BoardAccess, operations: ServeBoardOperation) -> Self {
        Self {
            serve,
            access,
            operations,
            feed: None,
            replication: None,
            standby: None,
            unavailable: std::sync::Mutex::default(),
        }
    }

    /// The routes, giving followers `replication`'s batches (B11).
    pub fn with_replication(self, replication: Option<ServeReplication>) -> Self {
        Self {
            replication,
            ..self
        }
    }

    /// The routes of a follower of `primary` (B11): every write is 409.
    pub fn standby(self, primary: String) -> Self {
        Self {
            standby: Some(primary),
            ..self
        }
    }

    /// A follower's refusal of a device's request while its copy is not
    /// kept current: 503 `standby_stale`. Its status (which says why) and
    /// time, and the Unix socket, are served still; its batches are not, so
    /// a follower chained to it stalls too.
    fn stale(&self, route: Route, transport: Transport) -> Option<ServiceReply> {
        match (route, transport, self.serve.serves_devices()) {
            (Route::Status | Route::Time, _, _) | (_, Transport::Local, _) | (_, _, true) => None,
            (_, Transport::Tcp, false) => {
                let primary = self.standby.as_deref().unwrap_or_default();
                let primary = redact_secrets(&redact_url_userinfo(primary));
                let why = "this standby's copy is not kept current (it is stalled, diverged, or \
                           not yet caught up): read its primary";
                let body = json!({"error": why, "code": "standby_stale", "primary": primary});
                Some(ServiceReply::json(503, &body))
            }
        }
    }

    /// A follower's refusal of a write: 409 `standby`, naming its primary.
    fn refused_standby(primary: &str) -> ServiceReply {
        let primary = redact_secrets(&redact_url_userinfo(primary));
        let body = json!({"error": "this server is a standby: write to its primary",
            "code": "standby", "primary": primary});
        ServiceReply::json(409, &body)
    }

    /// The routes, serving `feed` as the board's change feed (B10).
    pub fn with_feed(self, feed: Option<FollowBoard>) -> Self {
        Self { feed, ..self }
    }

    /// The feed of the board `path` names, when it is this server's; or
    /// the reply refusing it (404 `board`, or `route` with no feed).
    fn feed_of(&self, path: &str) -> Result<&FollowBoard, ServiceReply> {
        self.served(path)?;
        match &self.feed {
            Some(feed) => Ok(feed),
            None => Err(coded(404, "route", "no such route")),
        }
    }

    /// `Ok` when `path` names this server's board; else 404 `board`.
    fn served(&self, path: &str) -> Result<(), ServiceReply> {
        match board_of(path) {
            Some((board, _)) if board == *self.serve.board() => Ok(()),
            Some(_) | None => Err(coded(404, "board", "this server serves no such board")),
        }
    }

    /// The caller's principal and the route it may reach, or the reply
    /// refusing it: a route it names, a method it takes, a scope it holds.
    fn admitted(&self, request: &ServiceRequest) -> Result<(Route, Principal), ServiceReply> {
        let principal = match (request.transport, request.principal.as_deref()) {
            (Transport::Local, _) => Ok(Principal::Local),
            (Transport::Tcp, Some(name)) => BoardAccess::device(name),
            (Transport::Tcp, None) => Err(Refused::Token),
        };
        let principal = principal.map_err(|refused| self.refusal(&refused, request.transport))?;
        match Route::of(&request.path) {
            Some(route) if route.takes(request.method) => {
                match BoardAccess::authorize(&principal, route.scope()) {
                    Ok(()) => Ok((route, principal)),
                    Err(refused) => Err(self.refusal(&refused, request.transport)),
                }
            }
            Some(route) => Err(ServiceReply {
                allow: Some(route.allow()),
                ..coded(405, "method", "method not allowed")
            }),
            None => Err(coded(404, "route", "no such route")),
        }
    }

    /// An op's reply, or its refusal by its code.
    fn op(&self, request: &ServiceRequest, principal: &Principal) -> ServiceReply {
        if let Err(reply) = self.served(&request.path) {
            return reply;
        }
        let standby = self.standby.as_deref();
        match answer_op(
            &self.operations,
            principal,
            &request.body,
            standby.is_some(),
        ) {
            Ok(answer) => ServiceReply {
                body: answer.into_bytes(),
                ..ServiceReply::json(200, &serde_json::Value::Null)
            },
            Err(OpRefusal::Body(why)) => coded(400, "request", &why),
            Err(OpRefusal::Access(refused)) => self.refusal(&refused, request.transport),
            Err(OpRefusal::Standby) => Self::refused_standby(standby.unwrap_or_default()),
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
            Refused::ReplicaOnly => coded(
                403,
                "replica_only",
                "forbidden: replication batches are served to a replica token \
                 (`token add <name> --replica`) or on the Unix socket only",
            ),
            Refused::ReadOnly => coded(
                403,
                "replica_reads",
                "forbidden: a replica token reads and replicates, and writes nothing",
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
        // A follower writes nothing but its primary's batches: an allowlist
        // of the routes it answers whole (an op is judged by its scope).
        let read_only = matches!(
            (route, request.method),
            (Route::Tokens, ServiceMethod::Get)
                | (
                    Route::Status
                        | Route::Time
                        | Route::Op
                        | Route::Events
                        | Route::Revisions
                        | Route::Replicate,
                    _
                )
        );
        if let (Some(primary), false) = (&self.standby, read_only) {
            return Self::refused_standby(primary);
        }
        if let Some(stale) = self.stale(route, request.transport) {
            return stale;
        }
        match (route, request.method) {
            (Route::Op, _) => self.op(request, principal),
            (Route::Replicate, _) => match (self.served(&request.path), &self.replication) {
                (Ok(()), Some(replication)) => super::feed::replicate(replication, request),
                (Ok(()), None) => coded(404, "route", "no such route"),
                (Err(reply), _) => reply,
            },
            (Route::Revisions, _) => match self.feed_of(&request.path) {
                Ok(feed) => super::feed::revisions(feed, request),
                Err(reply) => reply,
            },
            // Never reached through the listener, which hands every GET of
            // `events` to `stream` (`streams` says so, and the route takes GET
            // alone). It stays for a caller of `answer` itself, such as a test
            // or a future front end, which is told to read the feed as a stream.
            (Route::Events, _) => coded(400, "stream", "the change feed is read as a stream"),
            (Route::Status, _) => read(self.serve.status().map(|status| {
                let mut body = json!({
                    "role": status.role.as_str(),
                    "board": status.board.as_str(),
                    "head": status.head.map(|head| head.0),
                });
                if let Some(replica) = status.replica {
                    let primary = redact_secrets(&redact_url_userinfo(&replica.primary));
                    body["replication"] = json!({
                        "primary": primary,
                        "applied": replica.applied,
                        "primary_head": replica.primary_head,
                        "state": replica.state.as_str(),
                        "reason": replica.state.reason().map(redact_secrets),
                    });
                }
                body
            })),
            (Route::Time, _) => read(
                self.serve
                    .time()
                    .map(|time| json!({ "time": time.as_str() })),
            ),
            (Route::Tokens, ServiceMethod::Get) => administer(self.access.list().map(|records| {
                let tokens: Vec<_> = records
                    .iter()
                    .map(|token| {
                        json!({"name": token.name.as_str(),
                        "created": token.created.as_str(), "kind": token.kind.as_str()})
                    })
                    .collect();
                (200, json!({ "tokens": tokens }))
            })),
            (Route::Tokens, ServiceMethod::Post) => {
                let Ok(adding) = serde_json::from_slice::<Adding>(&request.body) else {
                    let why = "the body must be {\"name\": \"<token name>\"} \
                               (and \"replica\": true for a follower's)";
                    return coded(400, "request", why);
                };
                let kind = match adding.replica {
                    true => TokenKind::Replica,
                    false => TokenKind::Device,
                };
                administer(self.access.add_kind(&adding.name, kind).map(|issued| {
                    let record = &issued.record;
                    let body = json!({"name": record.name.as_str(),
                        "created": record.created.as_str(), "kind": record.kind.as_str(),
                        "token": issued.secret});
                    (201, body)
                }))
            }
            (Route::Revoke, _) => named(&request.body, |name| {
                administer(
                    self.access
                        .revoke(name)
                        .map(|name| (200, json!({ "name": name.as_str(), "revoked": true }))),
                )
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
        let principal = self
            .access
            .authenticate_principal(authorization.and_then(bearer));
        let principal = principal.map_err(|refused| self.refusal(&refused, Transport::Tcp))?;
        Ok(principal.gated())
    }

    fn answer(&self, request: &ServiceRequest) -> ServiceReply {
        match self.admitted(request) {
            Ok((route, principal)) => self.route(route, request, &principal),
            Err(reply) => reply,
        }
    }

    fn allowed(&self, path: &str) -> Option<&'static str> {
        Route::of(path).map(Route::allow)
    }

    fn streams(&self, path: &str) -> bool {
        Route::of(path) == Some(Route::Events)
    }

    fn stream(
        &self,
        request: &ServiceRequest,
        wake: StreamWake,
    ) -> Result<Box<dyn ServiceStream>, ServiceReply> {
        let admitted = self.admitted(request)?;
        if let Some(stale) = self.stale(admitted.0, request.transport) {
            return Err(stale);
        }
        match admitted {
            (Route::Events, _) => super::feed::open(self.feed_of(&request.path)?, request, wake),
            (_, _) => Err(coded(404, "route", "no such route")),
        }
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
pub(super) fn failed(error: &ServeError) -> ServiceReply {
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
        ServeError::Reading => (
            503,
            "busy",
            "the remote is being read for another write: try again shortly".into(),
        ),
    };
    coded(status, code, &shown(&why))
}

#[cfg(test)]
#[path = "routes_tests.rs"]
pub(super) mod tests;

#[cfg(test)]
#[path = "routes_follower_tests.rs"]
mod follower_tests;
