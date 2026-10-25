//! The board service's blocking client (epic #2705 B4, B7): one request a
//! connection, a bounded reply; a TCP token no error or `Debug` shows.
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::limits::{MAX_HEAD_BYTES, Method};

/// The most bytes a reply's body may be: a whole board's listing.
pub const MAX_REPLY_BYTES: usize = 64 * 1024 * 1024;
/// How long a TCP connect may wait.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a read's reply may take.
pub const READ_REPLY: Duration = Duration::from_secs(30);
/// How long a write's reply may take: past a server's worst write (30 s of
/// pull request reads, one 20 s `gh` read, a turn at the lock).
pub const WRITE_REPLY: Duration = Duration::from_secs(75);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    Unix(PathBuf),
    Tcp(SocketAddr),
}

impl std::fmt::Display for Endpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unix(path) => write!(f, "unix://{}", path.display()),
            Self::Tcp(address) => write!(f, "http://{address}"),
        }
    }
}

/// A device's token: 16 to 512 token68 characters. Never shown.
#[derive(Clone, PartialEq, Eq)]
pub struct BearerToken(String);

impl BearerToken {
    pub fn parse(text: &str) -> Option<Self> {
        let token68 = |byte: u8| byte.is_ascii_alphanumeric() || b"-._~+/=".contains(&byte);
        let shaped = (16..=512).contains(&text.len()) && text.bytes().all(token68);
        shaped.then(|| Self(text.to_string()))
    }
}

impl std::fmt::Debug for BearerToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BearerToken(<redacted>)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientReply {
    pub status: u16,
    pub body: Vec<u8>,
}

/// Why no reply was had.
#[derive(Debug)]
pub enum ClientError {
    Connect(io::Error),
    Request(String),
    Io(io::Error),
    Closed,
    /// Sent, but no reply came in time: a write may have landed.
    NoReply,
    Malformed(String),
}

impl ClientError {
    /// The request or its reply was lost on the way: a retry may land.
    pub fn transport(&self) -> bool {
        match self {
            Self::Connect(_) | Self::Io(_) | Self::Closed => true,
            Self::Request(_) | Self::NoReply | Self::Malformed(_) => false,
        }
    }
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Connect(error) => write!(f, "no board server answers: {error}"),
            Self::Request(why) => write!(f, "not sent: {why}"),
            Self::Io(error) => write!(f, "the board server's connection failed: {error}"),
            Self::Closed => f.write_str("the board server closed the connection unanswered"),
            Self::NoReply => f.write_str("the board server sent no reply in time"),
            Self::Malformed(why) => write!(f, "the board server's reply is malformed: {why}"),
        }
    }
}

impl std::error::Error for ClientError {}

#[derive(Debug, Clone)]
pub struct BoardServiceClient {
    endpoint: Endpoint,
    token: Option<BearerToken>,
}

impl BoardServiceClient {
    pub fn unix(socket: &Path) -> Self {
        Self::new(Endpoint::Unix(socket.to_path_buf()), None)
    }

    pub fn new(endpoint: Endpoint, token: Option<BearerToken>) -> Self {
        Self { endpoint, token }
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// `method path` with the JSON `body`: the reply within `reply`.
    ///
    /// # Errors
    /// A path not of visible ASCII, no server, or a failed, late or bad reply.
    pub fn send(
        &self,
        method: Method,
        path: &str,
        body: &[u8],
        reply: Duration,
    ) -> Result<ClientReply, ClientError> {
        let target = path.starts_with('/') && path.bytes().all(|b| b.is_ascii_graphic());
        if !target {
            return Err(ClientError::Request(format!(
                "{path:?} is not a path of visible ASCII starting with /"
            )));
        }
        let authorization = match &self.token {
            Some(BearerToken(token)) => format!("authorization: Bearer {token}\r\n"),
            None => String::new(),
        };
        let head = format!(
            "{} {path} HTTP/1.1\r\nhost: board\r\nconnection: close\r\n\
             content-type: application/json\r\ncontent-length: {}\r\n{authorization}\r\n",
            method.as_str(),
            body.len()
        );
        let reply = match &self.endpoint {
            Endpoint::Unix(socket) => {
                let stream = UnixStream::connect(socket).map_err(ClientError::Connect)?;
                let timed = stream.set_read_timeout(Some(reply));
                let timed = timed.and(stream.set_write_timeout(Some(reply)));
                exchange(timed.map(|()| stream), &head, body)
            }
            Endpoint::Tcp(address) => {
                let stream = TcpStream::connect_timeout(address, CONNECT_TIMEOUT);
                let stream = stream.map_err(ClientError::Connect)?;
                let timed = stream.set_read_timeout(Some(reply));
                let timed = timed.and(stream.set_write_timeout(Some(reply)));
                exchange(timed.map(|()| stream), &head, body)
            }
        }?;
        match reply.is_empty() {
            true => Err(ClientError::Closed),
            false => parse_reply(&reply),
        }
    }
}

/// Sends `head` and `body` on `stream`; the reply, to its end and bound.
fn exchange(
    stream: io::Result<impl Read + Write>,
    head: &str,
    body: &[u8],
) -> Result<Vec<u8>, ClientError> {
    let mut stream = stream.map_err(ClientError::Io)?;
    stream.write_all(head.as_bytes()).map_err(ClientError::Io)?;
    stream.write_all(body).map_err(ClientError::Io)?;
    let mut reply = Vec::new();
    let late = |error: io::Error| match error.kind() {
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => ClientError::NoReply,
        _ => ClientError::Io(error),
    };
    stream
        .take((MAX_HEAD_BYTES + MAX_REPLY_BYTES + 1) as u64)
        .read_to_end(&mut reply)
        .map_err(late)?;
    Ok(reply)
}

/// The reply `bytes` holds: a status, and a body of its `Content-Length`.
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
