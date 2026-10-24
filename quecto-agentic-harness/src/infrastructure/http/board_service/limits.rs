//! The board service's limits (epic #2705 B4), each an allowlist: `GET`
//! and `POST` (else 405); bodies of a declared `Content-Length` up to 1 MiB
//! (else 411 or 413; hyper's parser answers a malformed one 400, and too
//! many headers 431, both bodyless); 32 headers in 8 KiB; 10 s for a head,
//! which also ends an idle keep-alive, and 10 s for a body (408); 10 s for
//! a peer to take any reply byte ([`WriteDeadline`]); 64 connections at
//! once a listener (TCP: see below); and each connection at most 60 s or
//! 1,000 requests, then it
//! finishes what it answers within 15 s and closes. No peer holds a slot
//! for good.
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use hyper::server::conn::http1;
use hyper_util::rt::TokioTimer;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// The most headers a request may carry.
pub const MAX_HEADERS: usize = 32;
/// The most bytes a request's head may take (hyper's smallest buffer).
pub const MAX_HEAD_BYTES: usize = 8 * 1024;
/// The most bytes a request's body may declare.
pub const MAX_BODY_BYTES: u64 = 1024 * 1024;
/// How long a head, then a body, may take to arrive; and a write may wait.
pub const READ_TIMEOUT: Duration = Duration::from_secs(10);
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
/// The most connections at once; each one's longest life and most requests.
pub const MAX_CONNECTIONS: usize = 64;
/// The unauthenticated TCP connections one source may hold (one more
/// evicts the oldest), and the time a TCP connection has to authenticate.
pub const MAX_UNPROVEN_PER_SOURCE: usize = 8;
pub const FIRST_REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
pub const CONNECTION_LIFETIME: Duration = Duration::from_secs(60);
pub const MAX_REQUESTS: usize = 1_000;
/// How long a connection past its life may take to finish what it answers.
pub const SHUTDOWN_GRACE: Duration = Duration::from_secs(15);

/// The methods the service takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

/// Why a request is refused before its body is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Method,
    LengthRequired,
    TooLarge,
}

impl Refusal {
    /// The status it is answered with, its typed code, and what it says.
    pub fn verdict(self) -> (u16, &'static str, &'static str) {
        match self {
            Self::Method => (405, "method", "method not allowed"),
            Self::LengthRequired => (411, "length", "a body needs a Content-Length"),
            Self::TooLarge => (413, "too_large", "the body is past 1 MiB"),
        }
    }
}

/// What a head admitted: its method and its body's declared length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Admitted {
    pub method: Method,
    pub body_bytes: u64,
}

/// Judges a request's head: `method` as sent, the body length hyper read
/// from its `Content-Length` (hyper itself refuses one that is no number,
/// 400, and merges equal repeats), and whether it names a
/// `Transfer-Encoding`.
///
/// # Errors
/// The first limit it breaks, method first.
pub fn admit(
    method: &str,
    length: Option<u64>,
    transfer_encoding: bool,
) -> Result<Admitted, Refusal> {
    let method = match method {
        "GET" => Method::Get,
        "POST" => Method::Post,
        _ => return Err(Refusal::Method),
    };
    let body_bytes = match (transfer_encoding, length) {
        (false, Some(length)) => length,
        (true, _) | (false, None) => return Err(Refusal::LengthRequired),
    };
    match body_bytes <= MAX_BODY_BYTES {
        true => Ok(Admitted { method, body_bytes }),
        false => Err(Refusal::TooLarge),
    }
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

/// A connection whose writes fail `TimedOut` once the peer takes no byte
/// for `limit`: a stalled write starts the clock, any progress stops it.
pub struct WriteDeadline<S> {
    io: S,
    limit: Duration,
    stalled: Option<Pin<Box<tokio::time::Sleep>>>,
}

impl<S> WriteDeadline<S> {
    pub fn new(io: S, limit: Duration) -> Self {
        Self {
            io,
            limit,
            stalled: None,
        }
    }

    /// `polled`, or `TimedOut` once it has been pending past the limit.
    fn watched<T>(
        &mut self,
        cx: &mut Context<'_>,
        polled: Poll<io::Result<T>>,
    ) -> Poll<io::Result<T>> {
        let Poll::Pending = polled else {
            self.stalled = None;
            return polled;
        };
        let limit = self.limit;
        let clock = self
            .stalled
            .get_or_insert_with(|| Box::pin(tokio::time::sleep(limit)));
        match clock.as_mut().poll(cx) {
            Poll::Ready(()) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the peer took no reply bytes in time",
            ))),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for WriteDeadline<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().io).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for WriteDeadline<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        let polled = Pin::new(&mut this.io).poll_write(cx, buf);
        this.watched(cx, polled)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let polled = Pin::new(&mut this.io).poll_flush(cx);
        this.watched(cx, polled)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let polled = Pin::new(&mut this.io).poll_shutdown(cx);
        this.watched(cx, polled)
    }
}

#[cfg(test)]
#[path = "limits_tests.rs"]
mod tests;
