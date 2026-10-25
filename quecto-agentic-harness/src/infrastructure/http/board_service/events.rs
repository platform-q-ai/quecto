//! The board service's event-stream reader (epic #2705 B10): one `GET`,
//! its `text/event-stream` reply read as it comes, chunk by chunk, each
//! event handed on; every head, chunk and event under a bound, and every
//! read under the idle time (a live stream sends a keep-alive in it).
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use super::client::{
    BoardServiceClient, CONNECT_TIMEOUT, ClientError, ClientReply, Endpoint, MAX_REPLY_BYTES,
    parse_reply,
};
use super::limits::MAX_HEAD_BYTES;

/// The most bytes one event, all its lines, may take.
pub const MAX_EVENT_BYTES: usize = 8 * 1024;

/// One event as the stream sent it: its id, its name, its data lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamEvent {
    pub id: Option<String>,
    pub event: String,
    pub data: String,
}

/// A stream opened, or the server's reply refusing it.
pub enum Opened {
    Stream(EventReader),
    Refused(ClientReply),
}

/// The connection a stream is read from.
type Wire = BufReader<Box<dyn Read + Send>>;

/// An open stream's events.
pub struct EventReader {
    body: BufReader<Chunked>,
}

impl BoardServiceClient {
    /// `GET path` as an event stream, resumed after `last_event_id`; each
    /// read waits at most `idle`.
    ///
    /// # Errors
    /// A path not of visible ASCII, no server, or a failed, late or
    /// malformed reply.
    pub fn open_events(
        &self,
        path: &str,
        last_event_id: Option<u64>,
        idle: Duration,
    ) -> Result<Opened, ClientError> {
        if !(path.starts_with('/') && path.bytes().all(|b| b.is_ascii_graphic())) {
            return Err(ClientError::Request(format!(
                "{path:?} is not a path of visible ASCII starting with /"
            )));
        }
        let resume = last_event_id
            .map(|id| format!("last-event-id: {id}\r\n"))
            .unwrap_or_default();
        let head = format!(
            "GET {path} HTTP/1.1\r\nhost: board\r\nconnection: close\r\n\
             accept: text/event-stream\r\n{resume}{}\r\n",
            self.authorization()
        );
        let wire: Box<dyn Read + Send> = match self.endpoint() {
            Endpoint::Unix(socket) => {
                let stream = UnixStream::connect(socket).map_err(ClientError::Connect)?;
                let timed = stream.set_read_timeout(Some(idle));
                timed
                    .and(stream.set_write_timeout(Some(idle)))
                    .map_err(ClientError::Io)?;
                (&stream)
                    .write_all(head.as_bytes())
                    .map_err(ClientError::Io)?;
                Box::new(stream)
            }
            Endpoint::Tcp(address) => {
                let stream = TcpStream::connect_timeout(address, CONNECT_TIMEOUT);
                let stream = stream.map_err(ClientError::Connect)?;
                let timed = stream.set_read_timeout(Some(idle));
                timed
                    .and(stream.set_write_timeout(Some(idle)))
                    .map_err(ClientError::Io)?;
                (&stream)
                    .write_all(head.as_bytes())
                    .map_err(ClientError::Io)?;
                Box::new(stream)
            }
        };
        let mut wire = BufReader::new(wire);
        let head = read_head(&mut wire)?;
        let (status, headers) = parse_head(&head)?;
        let named = |name: &str| {
            headers
                .iter()
                .filter(|(header, _)| header.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.to_ascii_lowercase())
                .collect::<Vec<_>>()
        };
        match status {
            200 => {
                let streamed = named("content-type")
                    .iter()
                    .any(|value| value.starts_with("text/event-stream"));
                let chunked = named("transfer-encoding") == ["chunked"];
                match (streamed, chunked) {
                    (true, true) => Ok(Opened::Stream(EventReader {
                        body: BufReader::new(Chunked {
                            wire,
                            left: 0,
                            done: false,
                        }),
                    })),
                    (_, _) => Err(ClientError::Malformed(
                        "a 200 that is not a chunked text/event-stream".into(),
                    )),
                }
            }
            _ => {
                let mut whole = head;
                wire.take((MAX_REPLY_BYTES + 1) as u64)
                    .read_to_end(&mut whole)
                    .map_err(late)?;
                Ok(Opened::Refused(parse_reply(&whole)?))
            }
        }
    }
}

impl EventReader {
    /// The next event; `None` once the stream ends (an event cut short is
    /// dropped, never handed on).
    ///
    /// # Errors
    /// A read failed or waited past the idle time, or the stream is not
    /// one this reader reads.
    pub fn next_event(&mut self) -> Result<Option<StreamEvent>, ClientError> {
        if MAX_EVENT_BYTES > 0 {
            return Ok(None);
        }
        let malformed = |why: &str| ClientError::Malformed(why.to_string());
        let (mut id, mut event, mut data, mut size) = (None, None, None::<String>, 0);
        loop {
            let mut line = Vec::new();
            let bound = (MAX_EVENT_BYTES + 1) as u64;
            let read = (&mut self.body).take(bound).read_until(b'\n', &mut line);
            let read = read.map_err(late)?;
            size += read;
            match (read, size <= MAX_EVENT_BYTES, line.last() == Some(&b'\n')) {
                (0, _, _) => return Ok(None),
                (_, false, _) => return Err(malformed("an event past its bound")),
                (_, true, true) => {}
                // The stream ended mid-line.
                (_, true, false) => return Ok(None),
            }
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = String::from_utf8(line).map_err(|_| malformed("a line not UTF-8"))?;
            if line.is_empty() {
                match (event.take(), data.take()) {
                    (None, None) => size = 0,
                    (event, data) => {
                        return Ok(Some(StreamEvent {
                            id,
                            event: event.unwrap_or_else(|| "message".into()),
                            data: data.unwrap_or_default(),
                        }));
                    }
                }
                continue;
            }
            let (field, value) = line.split_once(':').unwrap_or((line.as_str(), ""));
            let value = value.strip_prefix(' ').unwrap_or(value);
            match field {
                // A comment: a keep-alive.
                "" => {}
                "id" => id = Some(value.to_string()),
                "event" => event = Some(value.to_string()),
                "data" => match &mut data {
                    Some(held) => {
                        held.push('\n');
                        held.push_str(value);
                    }
                    None => data = Some(value.to_string()),
                },
                "retry" => {}
                _ => return Err(malformed("a field this reader does not read")),
            }
        }
    }
}

/// A chunked body, decoded.
struct Chunked {
    wire: Wire,
    /// What is left of the chunk being read.
    left: usize,
    done: bool,
}

impl Read for Chunked {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let invalid = |why: &str| io::Error::new(io::ErrorKind::InvalidData, why.to_string());
        if self.done || buf.is_empty() {
            return Ok(0);
        }
        if self.left == 0 {
            let line = bounded_line(&mut self.wire, 64)?;
            let size = line.split(';').next().unwrap_or_default().trim();
            let hex = (1..=8).contains(&size.len()) && size.bytes().all(|b| b.is_ascii_hexdigit());
            let size = hex
                .then(|| usize::from_str_radix(size, 16).ok())
                .flatten()
                .ok_or_else(|| invalid("a chunk size that is not hex"))?;
            if size == 0 {
                // The last chunk: its trailers are not read.
                self.done = true;
                return Ok(0);
            }
            self.left = size;
        }
        let most = buf.len().min(self.left);
        let read = self.wire.read(&mut buf[..most])?;
        if read == 0 {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof));
        }
        self.left -= read;
        if self.left == 0 && !bounded_line(&mut self.wire, 2)?.is_empty() {
            return Err(invalid("a chunk not ended by CRLF"));
        }
        Ok(read)
    }
}

/// One CRLF-ended line of at most `most` bytes, without its CRLF.
fn bounded_line(wire: &mut Wire, most: usize) -> io::Result<String> {
    let mut line = Vec::new();
    wire.take((most + 2) as u64).read_until(b'\n', &mut line)?;
    match line.strip_suffix(b"\r\n") {
        Some(text) => String::from_utf8(text.to_vec())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "a line not UTF-8")),
        None => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "a line not ended by CRLF in its bound",
        )),
    }
}

/// The reply's head, to its blank line, within [`MAX_HEAD_BYTES`].
fn read_head(wire: &mut Wire) -> Result<Vec<u8>, ClientError> {
    let mut head = Vec::new();
    loop {
        let room = (MAX_HEAD_BYTES + 1).saturating_sub(head.len()) as u64;
        let read = wire.take(room).read_until(b'\n', &mut head).map_err(late)?;
        match (
            read,
            head.ends_with(b"\r\n\r\n"),
            head.len() <= MAX_HEAD_BYTES,
        ) {
            (0, _, _) if head.is_empty() => return Err(ClientError::Closed),
            (_, true, true) => return Ok(head),
            (0, _, _) | (_, _, false) => {
                return Err(ClientError::Malformed("no end of head within 8 KiB".into()));
            }
            (_, false, true) => {}
        }
    }
}

/// A head's status and its headers.
fn parse_head(head: &[u8]) -> Result<(u16, Vec<(String, String)>), ClientError> {
    let malformed = |why: &str| ClientError::Malformed(why.to_string());
    let text = std::str::from_utf8(head).map_err(|_| malformed("a head not UTF-8"))?;
    let mut lines = text.trim_end().split("\r\n");
    let status = lines
        .next()
        .and_then(|line| line.strip_prefix("HTTP/1.1 "))
        .and_then(|rest| rest.get(..3))
        .filter(|code| code.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| malformed("no HTTP/1.1 status line"))?;
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_string(), value.trim().to_string()))
        .collect();
    Ok((status, headers))
}

/// A read's failure; one past its time is no reply.
fn late(error: io::Error) -> ClientError {
    match error.kind() {
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => ClientError::NoReply,
        _ => ClientError::Io(error),
    }
}

#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;
