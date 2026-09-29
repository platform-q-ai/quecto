//! Connection handling for the loopback OAuth callback listener (PR #2309).
//!
//! The listener needs only the request line, so only the line is framed and
//! capped: it ends at the first LF, with an optional CR before it (RFC 9112
//! section 2.2 allows a bare LF). The rest of the head (a browser sends every
//! `localhost` cookie, whatever the port) is scanned and discarded, never
//! buffered. Every read is bounded by the connection's own budget and by the
//! login deadline, and every answer is followed by a half-close and a drain
//! bounded in time and bytes, so closing never resets the connection before
//! the client has read the answer.

use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::Instant;

/// Size of each socket read.
const READ_CHUNK_BYTES: usize = 4096;

/// The listener's per-connection limits, injectable so tests stay fast.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CallbackLimits {
    /// Largest request line, its line ending included. A real callback line
    /// (path, code, state, scope) is well under 2 KiB.
    pub(super) request_line_bytes: usize,
    /// Most head bytes (line plus headers, through the blank line's LF) a
    /// connection may send; it is never read past. Browsers cap a request
    /// head near 256 KiB; these bytes are discarded.
    pub(super) head_bytes: usize,
    /// How long one connection may take to deliver what the listener needs,
    /// counted from its accept. It never outlasts the login deadline.
    pub(super) connection_budget: Duration,
    /// After answering, how long the listener keeps reading at most.
    pub(super) linger: Duration,
    /// After answering, how many bytes the listener reads at most.
    pub(super) linger_bytes: usize,
}

impl CallbackLimits {
    pub(super) const PRODUCTION: Self = Self {
        request_line_bytes: 8 * 1024,
        head_bytes: 256 * 1024,
        connection_budget: Duration::from_secs(5),
        linger: Duration::from_secs(1),
        linger_bytes: 64 * 1024,
    };
}

pub(super) const LINE_TOO_LONG_RESPONSE: &str = "HTTP/1.1 414 URI Too Long\r\nConnection: close\r\nContent-Length: 21\r\n\r\nRequest line too long";
pub(super) const HEAD_TOO_LARGE_RESPONSE: &str = "HTTP/1.1 431 Request Header Fields Too Large\r\nConnection: close\r\nContent-Length: 17\r\n\r\nHeaders too large";
pub(super) const INCOMPLETE_REQUEST_RESPONSE: &str =
    "HTTP/1.1 400 Bad Request\r\nConnection: close\r\nContent-Length: 18\r\n\r\nIncomplete request";

/// Outcome of reading part of a request from a callback connection.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Framed<T> {
    /// What was asked for arrived in full.
    Ready(T),
    /// This connection cannot be served: answer it with this response and
    /// go on to the next connection.
    Refused(&'static str),
    /// The login deadline passed: the login is over.
    LoginTimedOut,
}

/// One accepted callback connection.
pub(super) struct CallbackConnection<'a, S> {
    stream: S,
    limits: &'a CallbackLimits,
    login_deadline: Instant,
    /// Reads end here: the connection budget or the login deadline.
    read_until: Instant,
    /// Bytes read past the request line, not yet scanned.
    pending: Vec<u8>,
    /// Head bytes read so far, request line included.
    head_read: usize,
}

/// Outcome of one bounded read.
enum Read {
    Bytes(usize),
    /// End of stream or a read error: nothing more will arrive.
    Ended,
    BudgetSpent,
    LoginTimedOut,
}

impl<'a, S> CallbackConnection<'a, S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    pub(super) fn new(stream: S, limits: &'a CallbackLimits, login_deadline: Instant) -> Self {
        assert!(
            limits.request_line_bytes <= limits.head_bytes,
            "the request line is part of the head, so its cap fits the head's"
        );
        let read_until = std::cmp::min(login_deadline, Instant::now() + limits.connection_budget);
        Self {
            stream,
            limits,
            login_deadline,
            read_until,
            pending: Vec::new(),
            head_read: 0,
        }
    }

    /// Read the request line, without its line ending.
    pub(super) async fn request_line(&mut self) -> Framed<String> {
        let cap = self.limits.request_line_bytes;
        let mut line: Vec<u8> = Vec::with_capacity(READ_CHUNK_BYTES);
        let mut chunk = [0u8; READ_CHUNK_BYTES];
        loop {
            if let Some(lf) = line.iter().position(|byte| *byte == b'\n') {
                // The line, its LF included, must fit the cap.
                if lf < cap {
                    self.pending = line.split_off(lf + 1);
                    line.truncate(lf);
                    if line.last() == Some(&b'\r') {
                        line.pop();
                    }
                    assert!(line.len() < cap, "a framed line fits its cap");
                    return Framed::Ready(String::from_utf8_lossy(&line).into_owned());
                }
                return Framed::Refused(LINE_TOO_LONG_RESPONSE);
            }
            if line.len() >= cap {
                return Framed::Refused(LINE_TOO_LONG_RESPONSE);
            }
            match self.read(&mut chunk).await {
                Read::Bytes(n) => line.extend_from_slice(&chunk[..n]),
                Read::Ended | Read::BudgetSpent => {
                    return Framed::Refused(INCOMPLETE_REQUEST_RESPONSE);
                }
                Read::LoginTimedOut => return Framed::LoginTimedOut,
            }
        }
    }

    /// Read, and discard, the rest of the head through the blank line that
    /// ends it. Call only after [`Self::request_line`] returned `Ready`.
    pub(super) async fn rest_of_head(&mut self) -> Framed<()> {
        let mut scan = HeadScan::AtLineStart;
        let mut bytes = std::mem::take(&mut self.pending);
        let mut chunk = [0u8; READ_CHUNK_BYTES];
        loop {
            // `bytes` are the last head bytes read, so the first of them is
            // head byte `head_read - bytes.len() + 1`.
            assert!(bytes.len() <= self.head_read, "scanned bytes were read");
            let first_position = self.head_read - bytes.len() + 1;
            for (index, byte) in bytes.iter().enumerate() {
                scan = scan.next(*byte);
                match scan {
                    HeadScan::Ended => {
                        // The head, its blank line's LF included, must fit.
                        let position = first_position + index;
                        if position <= self.limits.head_bytes {
                            self.pending = bytes.split_off(index + 1);
                            return Framed::Ready(());
                        }
                        return Framed::Refused(HEAD_TOO_LARGE_RESPONSE);
                    }
                    HeadScan::AtLineStart | HeadScan::AfterLineStartCr | HeadScan::InLine => {}
                }
            }
            // The cap's worth was read and the head has not ended.
            if self.head_read >= self.limits.head_bytes {
                return Framed::Refused(HEAD_TOO_LARGE_RESPONSE);
            }
            match self.read(&mut chunk).await {
                Read::Bytes(n) => bytes = chunk[..n].to_vec(),
                Read::Ended | Read::BudgetSpent => {
                    return Framed::Refused(INCOMPLETE_REQUEST_RESPONSE);
                }
                Read::LoginTimedOut => return Framed::LoginTimedOut,
            }
        }
    }

    /// Write `response`, half-close, and drain what the client still sends
    /// within the linger limits, so closing does not reset the connection
    /// before the client has read the answer.
    pub(super) async fn answer(mut self, response: &str) {
        match self.stream.write_all(response.as_bytes()).await {
            Ok(()) => {}
            Err(_) => return,
        }
        match self.stream.shutdown().await {
            Ok(()) => {}
            Err(_) => return,
        }
        let until = std::cmp::min(self.login_deadline, Instant::now() + self.limits.linger);
        drain(&mut self.stream, until, self.limits.linger_bytes).await;
    }

    /// Read head bytes into `chunk`, never past the head cap: each read
    /// asks for at most what the cap still allows. Call only while the cap
    /// allows at least one more byte.
    async fn read(&mut self, chunk: &mut [u8]) -> Read {
        assert!(
            self.head_read < self.limits.head_bytes,
            "a read is asked for only while the head cap allows one"
        );
        let want = std::cmp::min(chunk.len(), self.limits.head_bytes - self.head_read);
        let buffer = &mut chunk[..want];
        let read = tokio::time::timeout_at(self.read_until, self.stream.read(buffer)).await;
        match read {
            Ok(Ok(n)) if n > 0 => {
                assert!(n <= want, "a read cannot exceed its buffer");
                self.head_read += n;
                assert!(
                    self.head_read <= self.limits.head_bytes,
                    "no read goes past the head cap"
                );
                Read::Bytes(n)
            }
            Ok(Ok(_)) | Ok(Err(_)) => Read::Ended,
            Err(_) if self.read_until < self.login_deadline => Read::BudgetSpent,
            Err(_) => Read::LoginTimedOut,
        }
    }
}

/// Read and discard what `reader` sends until it ends, `until` passes, or
/// `max_bytes` have been read. Returns how many bytes were discarded.
pub(super) async fn drain<R>(reader: &mut R, until: Instant, max_bytes: usize) -> usize
where
    R: AsyncRead + Unpin,
{
    let mut chunk = [0u8; READ_CHUNK_BYTES];
    let mut drained = 0usize;
    while drained < max_bytes {
        let want = std::cmp::min(chunk.len(), max_bytes - drained);
        match tokio::time::timeout_at(until, reader.read(&mut chunk[..want])).await {
            Ok(Ok(n)) if n > 0 => drained += n,
            Ok(Ok(_)) | Ok(Err(_)) | Err(_) => break,
        }
    }
    assert!(
        drained <= max_bytes,
        "the drain never exceeds its byte limit"
    );
    drained
}

/// Where a scan of header bytes stands: the head ends at an empty line,
/// which is an LF, or a CR and an LF, at the start of a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeadScan {
    AtLineStart,
    AfterLineStartCr,
    InLine,
    Ended,
}

impl HeadScan {
    fn next(self, byte: u8) -> Self {
        match (self, byte) {
            (Self::AtLineStart | Self::AfterLineStartCr, b'\n') => Self::Ended,
            (Self::AtLineStart, b'\r') => Self::AfterLineStartCr,
            (Self::InLine, b'\n') => Self::AtLineStart,
            (Self::AtLineStart | Self::AfterLineStartCr | Self::InLine, _) => Self::InLine,
            (Self::Ended, _) => Self::Ended,
        }
    }
}

#[cfg(test)]
#[path = "oauth_callback_request_tests.rs"]
mod tests;
