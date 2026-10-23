//! SPIKE (board service): the smallest HTTP/1.1 that serves the board,
//! over TCP (Tailscale) and a Unix socket alike: one request per
//! connection (`Connection: close`), JSON bodies, and a server-sent event
//! stream for the change feed. std only, so a CLI call pays no runtime
//! start-up.
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

const MAX_HEAD: usize = 64 * 1024;
const MAX_BODY: usize = 64 * 1024 * 1024;

/// Where a board server listens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    Tcp(String),
    Unix(PathBuf),
}

impl Endpoint {
    /// `http://host:port` or `unix:/path`.
    pub fn parse(text: &str) -> Option<Self> {
        if let Some(path) = text.strip_prefix("unix:") {
            return Some(Self::Unix(PathBuf::from(path)));
        }
        let rest = text.strip_prefix("http://")?.trim_end_matches('/');
        rest.contains(':').then(|| Self::Tcp(rest.to_string()))
    }

    pub fn text(&self) -> String {
        match self {
            Self::Tcp(address) => format!("http://{address}"),
            Self::Unix(path) => format!("unix:{}", path.display()),
        }
    }
}

pub trait Stream: Read + Write + Send {}
impl<T: Read + Write + Send> Stream for T {}

fn connect(endpoint: &Endpoint, timeout: Duration) -> std::io::Result<Box<dyn Stream>> {
    match endpoint {
        Endpoint::Tcp(address) => {
            let mut last = None;
            for addr in address.to_socket_addrs()? {
                match TcpStream::connect_timeout(&addr, timeout) {
                    Ok(stream) => {
                        stream.set_nodelay(true)?;
                        stream.set_read_timeout(Some(timeout))?;
                        stream.set_write_timeout(Some(timeout))?;
                        return Ok(Box::new(stream));
                    }
                    Err(error) => last = Some(error),
                }
            }
            Err(last.unwrap_or_else(|| std::io::Error::other("no address")))
        }
        Endpoint::Unix(path) => {
            let stream = UnixStream::connect(path)?;
            stream.set_read_timeout(Some(timeout))?;
            stream.set_write_timeout(Some(timeout))?;
            Ok(Box::new(stream))
        }
    }
}

/// A request as the server reads it.
#[derive(Debug)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub query: Vec<(String, String)>,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn query(&self, name: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// Reads one header block: the start line and headers.
fn read_head(reader: &mut impl BufRead) -> std::io::Result<(String, Vec<(String, String)>)> {
    let mut start = String::new();
    let mut total = reader.read_line(&mut start)?;
    if total == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "no request",
        ));
    }
    let mut headers = vec![];
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line)?;
        total += read;
        if read == 0 || total > MAX_HEAD {
            return Err(std::io::Error::other("header block cut short or too long"));
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_string(), value.trim().to_string()));
        }
    }
    Ok((start.trim_end().to_string(), headers))
}

fn read_body(reader: &mut impl Read, headers: &[(String, String)]) -> std::io::Result<Vec<u8>> {
    let length = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        .map(|(_, value)| value.parse::<usize>())
        .transpose()
        .map_err(std::io::Error::other)?;
    match length {
        Some(length) if length > MAX_BODY => Err(std::io::Error::other("body too large")),
        Some(length) => {
            let mut body = vec![0; length];
            reader.read_exact(&mut body)?;
            Ok(body)
        }
        None => {
            let mut body = vec![];
            reader.take(MAX_BODY as u64).read_to_end(&mut body)?;
            Ok(body)
        }
    }
}

fn read_request(reader: &mut impl BufRead) -> std::io::Result<Request> {
    let (start, headers) = read_head(reader)?;
    let mut parts = start.split(' ');
    let method = parts.next().unwrap_or_default().to_string();
    let target = parts.next().unwrap_or_default();
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let query = query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();
    let body = match method.as_str() {
        "POST" | "PUT" => read_body(reader, &headers)?,
        _ => vec![],
    };
    Ok(Request {
        method,
        path: path.to_string(),
        query,
        headers,
        body,
    })
}

/// Writes a whole response.
///
/// # Errors
/// The connection's.
pub fn respond(
    stream: &mut dyn Write,
    status: u16,
    body: &[u8],
    server: Duration,
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        409 => "Conflict",
        503 => "Service Unavailable",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
         X-Server-Micros: {}\r\nConnection: close\r\n\r\n",
        body.len(),
        server.as_micros()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

/// What a handler does with a request: answers it whole, or takes the
/// stream (the change feed).
pub type Handler = Arc<dyn Fn(Request, Box<dyn Stream>) + Send + Sync>;

fn serve_one(stream: Box<dyn Stream>, handler: &Handler) {
    let mut reader = BufReader::new(stream);
    let Ok(request) = read_request(&mut reader) else {
        return;
    };
    // Nothing is pipelined: the buffer holds no more than the request.
    let stream = reader.into_inner();
    handler(request, stream);
}

/// Serves `listener` (TCP) forever, a thread per connection.
pub fn serve_tcp(listener: TcpListener, handler: Handler) {
    for stream in listener.incoming().flatten() {
        let _ = stream.set_nodelay(true);
        let handler = Arc::clone(&handler);
        std::thread::spawn(move || serve_one(Box::new(stream), &handler));
    }
}

/// Serves `listener` (a Unix socket) forever, a thread per connection.
pub fn serve_unix(listener: UnixListener, handler: Handler) {
    for stream in listener.incoming().flatten() {
        let handler = Arc::clone(&handler);
        std::thread::spawn(move || serve_one(Box::new(stream), &handler));
    }
}

/// Why a call did not get an answer: the request may or may not have been
/// taken (`sent`), which an idempotent write's retry covers.
#[derive(Debug)]
pub struct CallError {
    pub sent: bool,
    pub why: String,
}

/// One call: status and body.
///
/// # Errors
/// The connection's, or a malformed answer.
pub fn call(
    endpoint: &Endpoint,
    method: &str,
    path: &str,
    body: Option<&[u8]>,
    timeout: Duration,
) -> Result<(u16, Vec<u8>), CallError> {
    let started = Instant::now();
    let unsent = |error: std::io::Error| CallError {
        sent: false,
        why: format!("{}: {error}", endpoint.text()),
    };
    let mut stream = connect(endpoint, timeout).map_err(unsent)?;
    let body = body.unwrap_or_default();
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: board\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).map_err(unsent)?;
    let sent = |error: std::io::Error| CallError {
        sent: true,
        why: format!("{}: {error}", endpoint.text()),
    };
    stream.write_all(body).map_err(sent)?;
    stream.flush().map_err(sent)?;
    let mut reader = BufReader::new(stream);
    let (status_line, headers) = read_head(&mut reader).map_err(sent)?;
    let status = status_line
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse::<u16>().ok())
        .ok_or_else(|| CallError {
            sent: true,
            why: format!("a malformed status line {status_line:?}"),
        })?;
    let answer = read_body(&mut reader, &headers).map_err(sent)?;
    let server = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("x-server-micros"))
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(0);
    super::timing::http(path, started.elapsed(), server);
    Ok((status, answer))
}

/// Opens an event stream at `path`: each `data:` line's payload, with the
/// event's name, handed to `each` until it answers false or the stream
/// ends.
///
/// # Errors
/// The connection's.
pub fn events(
    endpoint: &Endpoint,
    path: &str,
    idle: Duration,
    mut each: impl FnMut(&str, &str) -> bool,
) -> std::io::Result<()> {
    let mut stream = connect(endpoint, idle)?;
    let head = format!("GET {path} HTTP/1.1\r\nHost: board\r\nAccept: text/event-stream\r\n\r\n");
    stream.write_all(head.as_bytes())?;
    let mut reader = BufReader::new(stream);
    let (status, _) = read_head(&mut reader)?;
    if !status.contains(" 200 ") {
        return Err(std::io::Error::other(format!("feed answered {status}")));
    }
    let mut event = String::from("message");
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if let Some(name) = line.strip_prefix("event: ") {
            event = name.to_string();
        } else if let Some(data) = line.strip_prefix("data: ") {
            if !each(&event, data) {
                return Ok(());
            }
            event = String::from("message");
        }
    }
}
