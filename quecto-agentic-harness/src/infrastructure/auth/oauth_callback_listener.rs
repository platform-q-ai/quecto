//! The loopback OAuth callback listener's accept loop (PR #2309).

use super::oauth_callback_request::{CallbackLimits, RequestHead, read_request_head, reject};
use crate::domain::error::DomainError;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::time::Instant;

/// Serve `listener` until a valid callback on `callback_path` carrying
/// `expected_state` arrives, or `deadline` passes.
pub(super) async fn serve_oauth_callback(
    listener: TcpListener,
    callback_path: &str,
    expected_state: &str,
    deadline: Instant,
    limits: &CallbackLimits,
) -> Result<String, DomainError> {
    let expected = expected_state.to_string();

    loop {
        let accept = tokio::time::timeout_at(deadline, listener.accept()).await;
        let (mut stream, _) = match accept {
            Ok(Ok(conn)) => conn,
            Ok(Err(e)) => {
                return Err(DomainError::Provider(format!(
                    "callback accept error: {}",
                    e
                )));
            }
            Err(_) => {
                return Err(DomainError::Provider(
                    "OAuth callback timed out".to_string(),
                ));
            }
        };

        // Parse only a whole request head: one may arrive over several reads.
        let request = match read_request_head(&mut stream, deadline).await {
            RequestHead::Complete(head) => head,
            RequestHead::Rejected(response) => {
                reject(&mut stream, response, deadline, limits).await;
                continue;
            }
            RequestHead::TimedOut => {
                return Err(DomainError::Provider(
                    "OAuth callback timed out".to_string(),
                ));
            }
        };

        // Parse the GET <callback_path>?code=...&state=... line
        let first_line = request.lines().next().unwrap_or("");
        let path = first_line.split_whitespace().nth(1).unwrap_or("");

        // Exact path match on the registered redirect path (no prefixes
        // like /callbackevil or /callback/extra).
        let request_path = path.split('?').next().unwrap_or("");
        if request_path != callback_path {
            let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\n\r\nNot found";
            let _ = stream.write_all(resp.as_bytes()).await;
            continue;
        }

        // Parse query params (URL-decode values to handle encoded chars)
        let query = path.split('?').nth(1).unwrap_or("");
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
        let code = params.get("code").map(|s| s.as_str()).unwrap_or("");

        if state != expected {
            let resp = "HTTP/1.1 400 Bad Request\r\nContent-Length: 14\r\n\r\nState mismatch";
            let _ = stream.write_all(resp.as_bytes()).await;
            continue;
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
            let resp = "HTTP/1.1 400 Bad Request\r\nContent-Length: 12\r\n\r\nAuth failure";
            let _ = stream.write_all(resp.as_bytes()).await;
            return Err(DomainError::Provider(format!(
                "OAuth authorization failed: {}",
                if sanitized.is_empty() {
                    "unknown_error"
                } else {
                    &sanitized
                }
            )));
        }

        if code.is_empty() {
            let resp = "HTTP/1.1 400 Bad Request\r\nContent-Length: 12\r\n\r\nMissing code";
            let _ = stream.write_all(resp.as_bytes()).await;
            continue;
        }

        let html = "<html><body><h2>Authentication successful!</h2><p>You can close this window.</p></body></html>";
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{}",
            html.len(),
            html
        );
        let _ = stream.write_all(resp.as_bytes()).await;

        return Ok(code.to_string());
    }
}

#[cfg(test)]
#[path = "oauth_callback_listener_tests.rs"]
mod tests;
