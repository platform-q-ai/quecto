//! The loopback OAuth callback listener's accept loop (PR #2309).
//!
//! Connections are served one at a time. Each is answered, and dropped,
//! within its own budget ([`CallbackLimits::connection_budget`]): a stray,
//! slow or broken connection is refused and the listener goes on to the next
//! one. Only a valid callback, an OAuth error callback, or the login deadline
//! ends the login.

use super::oauth_callback_request::{CallbackConnection, CallbackLimits, Framed};
use crate::domain::error::DomainError;
use tokio::net::TcpListener;
use tokio::time::Instant;

const NOT_FOUND_RESPONSE: &str =
    "HTTP/1.1 404 Not Found\r\nConnection: close\r\nContent-Length: 9\r\n\r\nNot found";
const STATE_MISMATCH_RESPONSE: &str =
    "HTTP/1.1 400 Bad Request\r\nConnection: close\r\nContent-Length: 14\r\n\r\nState mismatch";
const MISSING_CODE_RESPONSE: &str =
    "HTTP/1.1 400 Bad Request\r\nConnection: close\r\nContent-Length: 12\r\n\r\nMissing code";
const AUTH_FAILURE_RESPONSE: &str =
    "HTTP/1.1 400 Bad Request\r\nConnection: close\r\nContent-Length: 12\r\n\r\nAuth failure";
const SUCCESS_HTML: &str = "<html><body><h2>Authentication successful!</h2><p>You can close this window.</p></body></html>";

/// What a request line asks of the listener.
#[derive(Debug, PartialEq, Eq)]
enum Route {
    /// Answer with this response and go on to the next connection.
    Declined(&'static str),
    /// The provider redirected with this authorization code.
    Authorized(String),
    /// The provider redirected with this (sanitized) OAuth error.
    Denied(String),
}

/// Serve `listener` until a valid callback on `callback_path` carrying
/// `expected_state` arrives, an OAuth error callback arrives, or
/// `login_deadline` passes.
pub(super) async fn serve_oauth_callback(
    listener: TcpListener,
    callback_path: &str,
    expected_state: &str,
    login_deadline: Instant,
    limits: &CallbackLimits,
) -> Result<String, DomainError> {
    // An empty expected state would accept a callback that carries none.
    assert!(
        !expected_state.is_empty(),
        "an OAuth login always expects a state"
    );
    loop {
        let accept = tokio::time::timeout_at(login_deadline, listener.accept()).await;
        let stream = match accept {
            Ok(Ok((stream, _))) => stream,
            Ok(Err(e)) => {
                return Err(DomainError::Provider(format!(
                    "callback accept error: {}",
                    e
                )));
            }
            Err(_) => return Err(timed_out()),
        };
        let mut connection = CallbackConnection::new(stream, limits, login_deadline);

        let line = match connection.request_line().await {
            Framed::Ready(line) => line,
            Framed::Refused(response) => {
                connection.answer(response).await;
                continue;
            }
            Framed::LoginTimedOut => return Err(timed_out()),
        };

        let route = route(&line, callback_path, expected_state);
        let outcome = match route {
            // Answered at once, as soon as the line is complete.
            Route::Declined(response) => {
                connection.answer(response).await;
                continue;
            }
            Route::Authorized(code) => Ok(code),
            Route::Denied(error) => Err(error),
        };

        // A login ends only on a whole request: a head cut off before its
        // blank line is refused, and the listener goes on.
        match connection.rest_of_head().await {
            Framed::Ready(()) => {}
            Framed::Refused(response) => {
                connection.answer(response).await;
                continue;
            }
            Framed::LoginTimedOut => return Err(timed_out()),
        }

        return match outcome {
            Ok(code) => {
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                    SUCCESS_HTML.len(),
                    SUCCESS_HTML
                );
                connection.answer(&response).await;
                Ok(code)
            }
            Err(error) => {
                connection.answer(AUTH_FAILURE_RESPONSE).await;
                Err(DomainError::Provider(format!(
                    "OAuth authorization failed: {}",
                    error
                )))
            }
        };
    }
}

fn timed_out() -> DomainError {
    DomainError::Provider("OAuth callback timed out".to_string())
}

/// Decide what the request line `GET <callback_path>?code=...&state=...`
/// asks for.
fn route(line: &str, callback_path: &str, expected_state: &str) -> Route {
    let target = line.split_whitespace().nth(1).unwrap_or("");

    // Exact path match on the registered redirect path (no prefixes
    // like /callbackevil or /callback/extra).
    let (request_path, query) = match target.split_once('?') {
        Some((path, query)) => (path, query),
        None => (target, ""),
    };
    if request_path != callback_path {
        return Route::Declined(NOT_FOUND_RESPONSE);
    }

    // Parse query params (URL-decode values to handle encoded chars)
    let params: std::collections::HashMap<String, String> = query
        .split('&')
        .filter_map(|p| {
            let (k, v) = p.split_once('=')?;
            Some((
                k.to_string(),
                urlencoding::decode(v).unwrap_or_default().into_owned(),
            ))
        })
        .collect();

    let state = params.get("state").map(|s| s.as_str()).unwrap_or("");
    if !constant_time_eq(state, expected_state) {
        return Route::Declined(STATE_MISMATCH_RESPONSE);
    }

    // Standard OAuth error callback (e.g. the user denied consent):
    // terminate immediately instead of waiting out the timeout.
    // Sanitize: only pass through short alphanumeric/underscore codes.
    if let Some(err) = params.get("error") {
        let sanitized: String = err
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
            .take(64)
            .collect();
        return Route::Denied(if sanitized.is_empty() {
            "unknown_error".to_string()
        } else {
            sanitized
        });
    }

    match params.get("code").map(|s| s.as_str()) {
        Some(code) if !code.is_empty() => Route::Authorized(code.to_string()),
        Some(_) | None => Route::Declined(MISSING_CODE_RESPONSE),
    }
}

/// Compare two strings in time that depends only on their lengths, so the
/// callback's state cannot be guessed byte by byte from response timing.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut diff = a.len() ^ b.len();
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= usize::from(x ^ y);
    }
    diff == 0
}

#[cfg(test)]
#[path = "oauth_callback_listener_tests.rs"]
mod tests;
