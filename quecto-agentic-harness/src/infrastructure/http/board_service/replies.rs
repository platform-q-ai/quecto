//! The board service's replies (epic #2705 B4, B10): a whole JSON reply,
//! or a stream a handler made, driven from its source as it is woken.
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full, StreamBody};
use hyper::body::{Bytes, Frame};
use hyper::{Response, StatusCode, header};
use tokio::sync::{Notify, mpsc};

use super::limits;
use super::listener::ServiceReply;
use super::sources::{Answers, StreamPlace};

/// A streamed reply's source (B10), asked on a blocking thread: once when
/// it is made, then each time its wake is called (or at once while it has
/// more); a keep-alive goes out while it is idle.
pub trait ServiceStream: Send {
    fn next(&mut self) -> StreamStep;
    /// The reply's `Content-Type`.
    fn content_type(&self) -> &'static str;
    /// What an idle stream sends to keep its reader's connection alive.
    fn keepalive(&self) -> &'static [u8];
}

/// What a stream gives: bytes to send (`more`: ask again at once), or
/// last bytes, after which it ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamStep {
    Send { bytes: Vec<u8>, more: bool },
    End(Vec<u8>),
}

/// Called, never blocking, when a stream has more to give.
pub type StreamWake = Arc<dyn Fn() + Send + Sync>;

/// A reply's body: whole, or streamed (B10).
pub(super) type ReplyBody = BoxBody<Bytes, Infallible>;

/// A stream the handler made, with what it holds while it runs.
pub(super) struct Streamed {
    pub(super) source: Box<dyn ServiceStream>,
    pub(super) woken: Arc<Notify>,
    pub(super) principal: Option<String>,
    pub(super) place: StreamPlace,
}

/// What a stream's driver reads of its connection: the TCP answers its
/// asks hold, when the connection's lifetime ends, and how often it pings.
pub(super) struct Driven {
    pub(super) answers: Arc<Answers>,
    pub(super) ends: tokio::time::Instant,
    pub(super) ping: Duration,
}

/// The reply a stream answers with: `200`, its `Content-Type`, no store,
/// its body sent as [`pump`] gives it.
pub(super) fn streaming(streamed: Streamed, driven: Driven) -> Response<ReplyBody> {
    let content_type = header::HeaderValue::from_static(streamed.source.content_type());
    let (out, mut taken) = mpsc::channel::<Bytes>(limits::STREAM_CHUNKS);
    tokio::spawn(pump(streamed, out, driven));
    let chunks = futures::stream::poll_fn(move |cx| {
        taken
            .poll_recv(cx)
            .map(|chunk| chunk.map(|bytes| Ok::<_, Infallible>(Frame::data(bytes))))
    });
    let mut response = Response::new(StreamBody::new(chunks).boxed());
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, content_type);
    let no_store = header::HeaderValue::from_static("no-store");
    headers.insert(header::CACHE_CONTROL, no_store);
    response
}

/// How long a stream waits to ask again while its TCP caller's blocking
/// answers are all held.
const STREAM_RETRY: Duration = Duration::from_millis(50);

/// Drives a stream until it ends, its reader goes, or its connection's
/// lifetime does: asks its source on a blocking thread (over TCP holding
/// one of its caller's answers) when woken, or at once while it has more,
/// and sends what it gives; a keep-alive each `ping` it is idle. A reader
/// that takes nothing stalls the send, the connection's write deadline then
/// drops the body, and this ends: the source's own queue is bounded, so
/// nothing it holds grows meanwhile.
async fn pump(streamed: Streamed, out: mpsc::Sender<Bytes>, conn: Driven) {
    let Streamed {
        mut source,
        woken,
        principal,
        place,
    } = streamed;
    let keepalive = Bytes::from_static(source.keepalive());
    let ends = conn.ends;
    let mut ask = true;
    while tokio::time::Instant::now() < ends {
        if !ask {
            tokio::select! {
                () = woken.notified() => {}
                () = tokio::time::sleep(conn.ping) => {
                    match sent(&out, keepalive.clone(), ends).await {
                        true => continue,
                        false => break,
                    }
                }
                () = tokio::time::sleep_until(ends) => break,
            }
        }
        let held = match principal.as_deref().map(|p| conn.answers.admit(p)) {
            None => None,
            Some(Some(held)) => Some(held),
            Some(None) => {
                tokio::time::sleep(STREAM_RETRY).await;
                ask = true;
                continue;
            }
        };
        let asked = tokio::task::spawn_blocking(move || {
            let step = source.next();
            drop(held);
            (source, step)
        });
        let Ok((back, step)) = asked.await else { break };
        source = back;
        let (bytes, more, end) = match step {
            StreamStep::Send { bytes, more } => (bytes, more, false),
            StreamStep::End(bytes) => (bytes, false, true),
        };
        let delivered = bytes.is_empty() || sent(&out, Bytes::from(bytes), ends).await;
        match (delivered, end) {
            (true, false) => ask = more,
            (true, true) | (false, _) => break,
        }
    }
    drop(place);
}

/// Whether `bytes` went to the body before `ends` (and its reader is there).
async fn sent(out: &mpsc::Sender<Bytes>, bytes: Bytes, ends: tokio::time::Instant) -> bool {
    tokio::select! {
        sent = out.send(bytes) => sent.is_ok(),
        () = tokio::time::sleep_until(ends) => false,
    }
}

/// A whole reply: its status, its JSON body and headers.
pub(super) fn response(reply: ServiceReply) -> Response<ReplyBody> {
    let status = StatusCode::from_u16(reply.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut response = Response::new(Full::new(Bytes::from(reply.body)).boxed());
    *response.status_mut() = status;
    let headers = response.headers_mut();
    let json = header::HeaderValue::from_static("application/json");
    headers.insert(header::CONTENT_TYPE, json);
    if let Some(allow) = reply.allow {
        headers.insert(header::ALLOW, header::HeaderValue::from_static(allow));
    }
    if let Some(challenge) = reply.challenge {
        let challenge = header::HeaderValue::from_static(challenge);
        headers.insert(header::WWW_AUTHENTICATE, challenge);
    }
    response
}
