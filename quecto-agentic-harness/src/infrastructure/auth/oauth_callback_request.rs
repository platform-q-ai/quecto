//! Request framing for the loopback OAuth callback listener (PR #2309).
//!
//! A browser's callback request can reach the listener in more than one
//! read: split across writes or TCP segments, or longer than one buffer. The
//! listener must therefore read up to the blank line that ends the headers
//! before it parses anything, within a byte cap and the login deadline.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::Instant;

/// Largest request head (request line plus headers, blank line included)
/// the listener accepts. A browser's callback head is a few KiB at most.
pub(super) const MAX_REQUEST_HEAD_BYTES: usize = 16 * 1024;

/// The byte sequence that ends an HTTP/1.x request head.
const HEAD_TERMINATOR: &[u8] = b"\r\n\r\n";

/// Size of each socket read while collecting the head.
const READ_CHUNK_BYTES: usize = 4096;

/// The listener's per-connection limits, injectable so tests stay fast.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CallbackLimits {
    /// Largest request line, its line ending included.
    pub(super) request_line_bytes: usize,
    /// Largest request head (line plus headers) read before a login ends.
    pub(super) head_bytes: usize,
    /// How long one connection may take to deliver what the listener needs.
    pub(super) connection_budget: std::time::Duration,
    /// After answering, how long the listener keeps reading so closing the
    /// socket with unread bytes does not reset the connection before the
    /// client has read the answer.
    pub(super) linger: std::time::Duration,
    /// After answering, how many bytes the listener reads at most.
    pub(super) linger_bytes: usize,
}

impl CallbackLimits {
    pub(super) const PRODUCTION: Self = Self {
        request_line_bytes: 8 * 1024,
        head_bytes: 256 * 1024,
        connection_budget: std::time::Duration::from_secs(5),
        linger: std::time::Duration::from_secs(1),
        linger_bytes: 64 * 1024,
    };
}

pub(super) const HEAD_TOO_LARGE_RESPONSE: &str =
    "HTTP/1.1 431 Request Header Fields Too Large\r\nContent-Length: 17\r\n\r\nHeaders too large";
pub(super) const INCOMPLETE_REQUEST_RESPONSE: &str =
    "HTTP/1.1 400 Bad Request\r\nContent-Length: 18\r\n\r\nIncomplete request";

/// Outcome of reading one request head from a callback connection.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum RequestHead {
    /// The whole head, through the blank line that ends it.
    Complete(String),
    /// The head cannot be served; answer with this response and move on.
    Rejected(&'static str),
    /// The login deadline passed before the head was complete.
    TimedOut,
}

/// Read one request head from `stream`, never past `deadline` and never more
/// than [`MAX_REQUEST_HEAD_BYTES`] of head.
pub(super) async fn read_request_head<S>(stream: &mut S, deadline: Instant) -> RequestHead
where
    S: AsyncRead + Unpin,
{
    let mut head: Vec<u8> = Vec::with_capacity(READ_CHUNK_BYTES);
    let mut chunk = [0u8; READ_CHUNK_BYTES];
    loop {
        // The overall deadline bounds every read: a client that connects and
        // sends nothing, or trickles bytes, must not block login forever
        // (PR #1087 review).
        let read = match tokio::time::timeout_at(deadline, stream.read(&mut chunk)).await {
            Ok(read) => read,
            Err(_) => return RequestHead::TimedOut,
        };
        // Only a read that delivered bytes continues the head; end of stream
        // or a read error before the blank line leaves the request incomplete.
        let n = match read {
            Ok(n) if n > 0 => n,
            _ => return RequestHead::Rejected(INCOMPLETE_REQUEST_RESPONSE),
        };
        assert!(n <= chunk.len(), "a read cannot exceed its buffer");
        // The terminator may straddle the previous read and this one.
        let scan_from = head.len().saturating_sub(HEAD_TERMINATOR.len() - 1);
        head.extend_from_slice(&chunk[..n]);
        if let Some(offset) = find_terminator(&head[scan_from..]) {
            let end = scan_from + offset + HEAD_TERMINATOR.len();
            assert!(end <= head.len(), "the head ends inside what was read");
            if end <= MAX_REQUEST_HEAD_BYTES {
                return RequestHead::Complete(String::from_utf8_lossy(&head[..end]).into_owned());
            }
            return RequestHead::Rejected(HEAD_TOO_LARGE_RESPONSE);
        }
        // Without a terminator yet, a head that has reached the cap can only
        // end past it.
        if head.len() >= MAX_REQUEST_HEAD_BYTES {
            return RequestHead::Rejected(HEAD_TOO_LARGE_RESPONSE);
        }
    }
}

/// Answer a rejected request, then close it gracefully: half-close, and drain
/// what the client is still sending (bounded by [`LINGER`], [`LINGER_BYTES`]
/// and `deadline`), so the kernel does not reset the connection over unread
/// bytes before the client has read `response`.
pub(super) async fn reject<S>(
    stream: &mut S,
    response: &str,
    deadline: Instant,
    limits: &CallbackLimits,
) where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if stream.write_all(response.as_bytes()).await.is_err() {
        return;
    }
    if stream.shutdown().await.is_err() {
        return;
    }
    let linger_until = std::cmp::min(deadline, Instant::now() + limits.linger);
    let mut chunk = [0u8; READ_CHUNK_BYTES];
    let mut drained = 0usize;
    while drained < limits.linger_bytes {
        match tokio::time::timeout_at(linger_until, stream.read(&mut chunk)).await {
            Ok(Ok(n)) if n > 0 => drained += n,
            _ => return,
        }
    }
}

fn find_terminator(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(HEAD_TERMINATOR.len())
        .position(|window| window == HEAD_TERMINATOR)
}

#[cfg(test)]
#[path = "oauth_callback_request_tests.rs"]
mod tests;
