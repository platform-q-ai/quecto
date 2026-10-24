//! The board service's limits (epic #2705 B4), each an allowlist: the
//! methods taken, bodies only of a declared length up to 1 MiB, at most
//! 32 headers in 8 KiB, 10 s to send a request's head or body, and 64
//! connections open at once. hyper's parser enforces the header limits
//! (431); [`admit`] judges a parsed head before any body is read.
use std::time::Duration;

use hyper::server::conn::http1;
use hyper_util::rt::TokioTimer;

/// The most headers a request may carry.
pub const MAX_HEADERS: usize = 32;
/// The most bytes a request's head may take (hyper's smallest buffer).
pub const MAX_HEAD_BYTES: usize = 8 * 1024;
/// The most bytes a request's body may declare.
pub const MAX_BODY_BYTES: u64 = 1024 * 1024;
/// How long a request's head, and then its body, may take to arrive.
pub const READ_TIMEOUT: Duration = Duration::from_secs(10);
/// The most connections served at once; a further peer waits to be
/// accepted.
pub const MAX_CONNECTIONS: usize = 64;

/// The methods the service takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

/// Why a request is refused before its body is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// A method outside the allowlist.
    Method,
    /// A body of no declared length (`Transfer-Encoding`).
    LengthRequired,
    /// A `Content-Length` that is not one plain decimal number.
    BadLength,
    /// A declared body past [`MAX_BODY_BYTES`].
    TooLarge,
}

impl Refusal {
    /// The status it is answered with.
    pub fn status(self) -> u16 {
        match self {
            Self::Method => 405,
            Self::LengthRequired => 411,
            Self::BadLength => 400,
            Self::TooLarge => 413,
        }
    }

    /// What its answer says.
    pub fn reason(self) -> &'static str {
        match self {
            Self::Method => "method not allowed: the board service takes GET and POST",
            Self::LengthRequired => "a body needs a Content-Length",
            Self::BadLength => "Content-Length is not a length",
            Self::TooLarge => "the body is past 1 MiB",
        }
    }
}

/// What a head admitted: its method and its body's declared length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Admitted {
    pub method: Method,
    pub body_bytes: u64,
}

/// Judges a request's head: `method` as sent, its `Content-Length`
/// values, and whether it names a `Transfer-Encoding`.
///
/// # Errors
/// The first limit it breaks, method first.
pub fn admit(
    method: &str,
    lengths: &[&[u8]],
    transfer_encoding: bool,
) -> Result<Admitted, Refusal> {
    let method = match method {
        "GET" => Method::Get,
        "POST" => Method::Post,
        _ => return Err(Refusal::Method),
    };
    if transfer_encoding {
        return Err(Refusal::LengthRequired);
    }
    let body_bytes = match lengths {
        [] => 0,
        [length] => decimal(length).ok_or(Refusal::BadLength)?,
        // Two lengths are refused, even equal ones: one is all it takes.
        _ => return Err(Refusal::BadLength),
    };
    match body_bytes <= MAX_BODY_BYTES {
        true => Ok(Admitted { method, body_bytes }),
        false => Err(Refusal::TooLarge),
    }
}

/// `bytes` as a plain decimal number of at most 19 digits (so it fits).
fn decimal(bytes: &[u8]) -> Option<u64> {
    let plain = (1..=19).contains(&bytes.len()) && bytes.iter().all(u8::is_ascii_digit);
    plain
        .then(|| std::str::from_utf8(bytes).ok()?.parse().ok())
        .flatten()
}

/// An HTTP/1.1 connection builder with the head limits and timeout set.
pub fn http1_builder() -> http1::Builder {
    let mut builder = http1::Builder::new();
    builder
        .timer(TokioTimer::new())
        .header_read_timeout(READ_TIMEOUT)
        .max_headers(MAX_HEADERS)
        .max_buf_size(MAX_HEAD_BYTES)
        .keep_alive(true);
    builder
}

#[cfg(test)]
#[path = "limits_tests.rs"]
mod tests;
