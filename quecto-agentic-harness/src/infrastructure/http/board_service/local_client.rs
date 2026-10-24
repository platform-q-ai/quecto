//! A blocking client of the board server's Unix socket for the admin
//! commands (epic #2705 B5); B7's remote client replaces it.
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;

use super::limits::{self, Method};

/// Sends `method path` with the JSON `body` on `socket`; the answer's
/// status and body, read whole within the limits.
/// # Errors
/// No server answers, too slowly, or not in HTTP.
pub fn local_request(
    socket: &Path,
    method: Method,
    path: &str,
    body: &[u8],
) -> io::Result<(u16, Vec<u8>)> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(limits::READ_TIMEOUT))?;
    stream.set_write_timeout(Some(limits::WRITE_TIMEOUT))?;
    let (method, length) = (method.as_str(), body.len());
    let head = format!(
        "{method} {path} HTTP/1.1\r\nHost: board\r\nConnection: close\r\n\
         Content-Type: application/json\r\nContent-Length: {length}\r\n\r\n"
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    let most = limits::MAX_BODY_BYTES + limits::MAX_HEAD_BYTES as u64;
    let mut answer = Vec::new();
    stream.take(most).read_to_end(&mut answer)?;
    parsed(&answer).ok_or_else(|| io::Error::other("the server's answer is not HTTP"))
}

/// An answer's status and body, when it is HTTP/1.1.
fn parsed(answer: &[u8]) -> Option<(u16, Vec<u8>)> {
    let split = answer.windows(4).position(|window| window == b"\r\n\r\n")?;
    let head = std::str::from_utf8(answer.get(..split)?).ok()?;
    let status = head.strip_prefix("HTTP/1.1 ")?.get(..3)?.parse().ok()?;
    Some((status, answer.get(split + 4..)?.to_vec()))
}

#[cfg(test)]
#[path = "local_client_tests.rs"]
mod tests;
