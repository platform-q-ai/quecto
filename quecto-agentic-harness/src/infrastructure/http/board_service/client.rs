//! The board service's blocking client (epic #2705 B4): one HTTP/1.1
//! request per connection over the server's Unix socket, with no runtime
//! to start, so the CLI reaches the server without tokio. TCP joins it
//! with B5.
//!
//! The reply is read within bounds: a head of at most 8 KiB, a body of
//! the declared `Content-Length` (required) up to [`MAX_REPLY_BYTES`],
//! each read within the client's timeout.
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::limits::MAX_HEAD_BYTES;

/// The most bytes a reply's body may be: a board snapshot's bound.
pub const MAX_REPLY_BYTES: usize = 128 * 1024 * 1024;
/// How long a request may wait on the socket, each read and write.
pub const TIMEOUT: Duration = Duration::from_secs(30);

/// A reply: its status and body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientReply {
    pub status: u16,
    pub body: Vec<u8>,
}

/// Why no reply was had.
#[derive(Debug)]
pub enum ClientError {
    /// No server answers on the socket.
    Connect(io::Error),
    /// A request the client does not send: why.
    Request(String),
    /// The connection failed mid-request.
    Io(io::Error),
    /// The reply is not one the client reads: why.
    Malformed(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connect(error) => write!(f, "no board server answers: {error}"),
            Self::Request(why) => write!(f, "not sent: {why}"),
            Self::Io(error) => write!(f, "the board server's connection failed: {error}"),
            Self::Malformed(why) => write!(f, "the board server's reply is malformed: {why}"),
        }
    }
}

impl std::error::Error for ClientError {}

/// A client of the board server on one Unix socket.
#[derive(Debug, Clone)]
pub struct BoardServiceClient {
    socket: PathBuf,
}

impl BoardServiceClient {
    pub fn unix(socket: &Path) -> Self {
        Self {
            socket: socket.to_path_buf(),
        }
    }

    /// `GET path`: the reply, whatever its status.
    ///
    /// # Errors
    /// A path that is not `/` and visible ASCII, no server, a failed
    /// connection, or a reply past its bounds or malformed.
    pub fn get(&self, path: &str) -> Result<ClientReply, ClientError> {
        let target = path.starts_with('/') && path.bytes().all(|b| b.is_ascii_graphic());
        if !target {
            return Err(ClientError::Request(format!(
                "{path:?} is not a path of visible ASCII starting with /"
            )));
        }
        let mut stream = UnixStream::connect(&self.socket).map_err(ClientError::Connect)?;
        stream
            .set_read_timeout(Some(TIMEOUT))
            .and_then(|()| stream.set_write_timeout(Some(TIMEOUT)))
            .map_err(ClientError::Io)?;
        let request = format!("GET {path} HTTP/1.1\r\nhost: board\r\nconnection: close\r\n\r\n");
        stream
            .write_all(request.as_bytes())
            .map_err(ClientError::Io)?;
        let mut reply = Vec::new();
        stream
            .take((MAX_HEAD_BYTES + MAX_REPLY_BYTES + 1) as u64)
            .read_to_end(&mut reply)
            .map_err(ClientError::Io)?;
        parse_reply(&reply)
    }
}

/// The reply `bytes` holds: an `HTTP/1.1 <3 digits>` status line, a head
/// within its bound, and a body of exactly its one `Content-Length`.
///
/// # Errors
/// What in it the client does not read.
pub fn parse_reply(bytes: &[u8]) -> Result<ClientReply, ClientError> {
    let malformed = |why: &str| ClientError::Malformed(why.to_string());
    let end = bytes
        .windows(4)
        .take(MAX_HEAD_BYTES)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| malformed("no end of head within 8 KiB"))?;
    let head = std::str::from_utf8(&bytes[..end]).map_err(|_| malformed("a head not UTF-8"))?;
    let body = &bytes[end + 4..];
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.strip_prefix("HTTP/1.1 "))
        .and_then(|rest| rest.get(..3))
        .filter(|code| code.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| malformed("no HTTP/1.1 status line"))?;
    let lengths: Vec<&str> = lines
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| name.trim().eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| value.trim())
        .collect();
    let length = match lengths.as_slice() {
        [length] if length.bytes().all(|b| b.is_ascii_digit()) => length
            .parse::<usize>()
            .map_err(|_| malformed("a Content-Length past its bound"))?,
        _ => return Err(malformed("not one Content-Length")),
    };
    match (length <= MAX_REPLY_BYTES, body.len() == length) {
        (true, true) => Ok(ClientReply {
            status,
            body: body.to_vec(),
        }),
        (false, _) => Err(malformed("a body past its bound")),
        (true, false) => Err(malformed("a body not of its Content-Length")),
    }
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
