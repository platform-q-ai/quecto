//! The board service's route table (epic #2705 B4), an allowlist: `GET
//! /v1/status` and `GET /v1/time` (else 404, or 405 naming the method a
//! route takes). The snapshot route is B11's.
use serde_json::json;

use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::{BoardStoreError, ServeError};
use crate::application::project_board::use_cases::ServeBoard;
use crate::domain::shared::redaction::{redact_secrets, redact_url_userinfo};
use crate::infrastructure::http::board_service::listener::{
    ServiceHandler, ServiceMethod, ServiceReply, ServiceRequest,
};

/// The routes, over the server's reads of its board.
pub struct BoardRoutes {
    serve: ServeBoard,
}

/// A route of the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Route {
    Status,
    Time,
}

impl Route {
    /// The route `path` names, if the table holds it.
    fn of(path: &str) -> Option<Self> {
        match path {
            "/v1/status" => Some(Self::Status),
            "/v1/time" => Some(Self::Time),
            _ => None,
        }
    }

    /// The one method it takes, and its `Allow` value.
    fn method(self) -> (ServiceMethod, &'static str) {
        match self {
            Self::Status | Self::Time => (ServiceMethod::Get, "GET"),
        }
    }
}

impl BoardRoutes {
    pub fn new(serve: ServeBoard) -> Self {
        Self { serve }
    }

    fn route(&self, route: Route) -> ServiceReply {
        let answered = match route {
            Route::Status => self.serve.status().map(|status| {
                json!({
                    "role": status.role.as_str(),
                    "board": status.board.as_str(),
                    "head": status.head.map(|head| head.0),
                })
            }),
            Route::Time => self
                .serve
                .time()
                .map(|time| json!({ "time": time.as_str() })),
        };
        match answered {
            Ok(body) => ServiceReply::json(200, &body),
            Err(error) => failed(&error),
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

/// An error reply with its typed `code`.
fn coded(status: u16, code: &str, why: &str) -> ServiceReply {
    ServiceReply::json(status, &json!({ "error": why, "code": code }))
}

/// A failed read's reply: 404 `no_board` while there is no board, else 503
/// `unavailable`; a store error may quote its remote, so it is redacted.
fn failed(error: &ServeError) -> ServiceReply {
    let (status, code, why) = match error {
        ServeError::Board(error @ ProjectBoardError::Store(BoardStoreError::NoBoard)) => {
            (404, "no_board", error.to_string())
        }
        ServeError::Board(error) => (503, "unavailable", error.to_string()),
    };
    coded(status, code, &redact_secrets(&redact_url_userinfo(&why)))
}

#[cfg(test)]
#[path = "routes_tests.rs"]
pub(super) mod tests;
