//! The board service's blocking client (epic #2705 B4, B7): one HTTP/1.1
//! request per connection, over the server's Unix socket or TCP, with no
//! runtime to start, so the CLI reaches the server without tokio. Over
//! TCP each request carries the device's bearer token, which no error,
//! log or `Debug` of the client ever shows.
//!
//! The reply is read within bounds: a head of at most 8 KiB, a body of
//! the declared `Content-Length` (required) up to [`MAX_REPLY_BYTES`],
//! each read and write within [`TIMEOUT`].
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::limits::{MAX_HEAD_BYTES, Method};

/// The most bytes a reply's body may be: a whole board's listing, with
/// room.
pub const MAX_REPLY_BYTES: usize = 64 * 1024 * 1024;
/// How long a request may wait on its connection, each read and write,
/// and a TCP connect.
pub const TIMEOUT: Duration = Duration::from_secs(30);

/// Where a board server answers: its Unix socket, or a TCP address.
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

/// A device's bearer token: 16 to 512 of the token68 characters, so it
/// can never break the header it is sent in. Never shown.
#[derive(Clone, PartialEq, Eq)]
pub struct BearerToken(String);

impl BearerToken {
    /// `text` as a token, when it is shaped as one.
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

/// A reply: its status and body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientReply {
    pub status: u16,
    pub body: Vec<u8>,
}

/// Why no reply was had.
#[derive(Debug)]
pub enum ClientError {
    /// No server answers at the endpoint.
    Connect(io::Error),
    /// A request the client does not send: why.
    Request(String),
    /// The connection failed mid-request.
    Io(io::Error),
    /// The server closed the connection without a byte of reply.
    Closed,
    /// The reply is not one the client reads: why.
    Malformed(String),
}

impl ClientError {
    /// The request may not have reached the server, or its reply was lost
    /// on the way: one a retry may be answered.
    pub fn transport(&self) -> bool {
        match self {
            Self::Connect(_) | Self::Io(_) | Self::Closed => true,
            Self::Request(_) | Self::Malformed(_) => false,
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
            Self::Malformed(why) => write!(f, "the board server's reply is malformed: {why}"),
        }
    }
}

impl std::error::Error for ClientError {}

/// A client of one board server.
#[derive(Debug, Clone)]
pub struct BoardServiceClient {
    endpoint: Endpoint,
    token: Option<BearerToken>,
}

impl BoardServiceClient {
    /// A client of the server on `socket` (no token: the socket's peer
    /// credentials let this user in).
    pub fn unix(socket: &Path) -> Self {
        Self::new(Endpoint::Unix(socket.to_path_buf()), None)
    }

    /// A client of `endpoint`, sending `token` on each request when given.
    pub fn new(endpoint: Endpoint, token: Option<BearerToken>) -> Self {
        Self { endpoint, token }
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    /// `method path` with the JSON `body`: the reply, whatever its status.
    ///
    /// # Errors
    /// A path that is not `/` and visible ASCII, no server, a failed
    /// connection, or a reply past its bounds or malformed.
    pub fn send(
        &self,
        method: Method,
        path: &str,
        body: &[u8],
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
            Endpoint::Unix(socket) => exchange(
                UnixStream::connect(socket).map_err(ClientError::Connect)?,
                &head,
                body,
            ),
            Endpoint::Tcp(address) => {
                let stream = TcpStream::connect_timeout(address, TIMEOUT);
                exchange(stream.map_err(ClientError::Connect)?, &head, body)
            }
        }?;
        match reply.is_empty() {
            true => Err(ClientError::Closed),
            false => parse_reply(&reply),
        }
    }
}

/// A stream the client times out on.
trait Timed: Read + Write {
    fn timeouts(&self, timeout: Duration) -> io::Result<()>;
}

impl Timed for UnixStream {
    fn timeouts(&self, timeout: Duration) -> io::Result<()> {
        self.set_read_timeout(Some(timeout))?;
        self.set_write_timeout(Some(timeout))
    }
}

impl Timed for TcpStream {
    fn timeouts(&self, timeout: Duration) -> io::Result<()> {
        self.set_read_timeout(Some(timeout))?;
        self.set_write_timeout(Some(timeout))
    }
}

/// Sends `head` and `body` on `stream` and reads the reply to its end,
/// within its bound.
fn exchange(mut stream: impl Timed, head: &str, body: &[u8]) -> Result<Vec<u8>, ClientError> {
    stream.timeouts(TIMEOUT).map_err(ClientError::Io)?;
    stream.write_all(head.as_bytes()).map_err(ClientError::Io)?;
    stream.write_all(body).map_err(ClientError::Io)?;
    let mut reply = Vec::new();
    stream
        .take((MAX_HEAD_BYTES + MAX_REPLY_BYTES + 1) as u64)
        .read_to_end(&mut reply)
        .map_err(ClientError::Io)?;
    Ok(reply)
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
