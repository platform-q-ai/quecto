//! The board service's listener (epic #2705 B4, B5): HTTP/1.1 on a Unix
//! socket and on TCP, parsed by hyper; it knows no board and no token. A
//! Unix peer is let in only as the uid the injected [`PeerCredentials`]
//! allow; a TCP request is first put to the injected [`ServiceHandler`]'s
//! gate. Each request is held to the [`limits`] and answered by the handler
//! on a blocking thread. [`bind`] makes the socket mode 0600 in a 0700
//! directory; [`bind_tcp`] binds the address it is given.
//!
//! Every reply this module or its handler makes is JSON with a typed
//! `code`. The HTTP layer's own refusals, made by hyper's parser before
//! any of it runs, have no body: clients map them by status, 400 as
//! `bad_request` and 431 as `head_too_large` (docs/board-server.md).
use std::convert::Infallible;
use std::fs;
use std::future::Future;
use std::io;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, PoisonError};
use std::time::Duration;

use http_body_util::{BodyExt, Full, Limited};
use hyper::body::{Bytes, Incoming};
use hyper::{Request, Response, StatusCode, header};
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UnixListener, UnixStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, watch};

use super::limits::{self, Refusal, WriteDeadline};
use super::sources::{Answers, Shares, Sources, Unproven, source_of};

pub use super::limits::Method as ServiceMethod;

/// A request as the handler gets it: admitted, its body read whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceRequest {
    pub method: ServiceMethod,
    /// The request target's path, without its query.
    pub path: String,
    pub body: Vec<u8>,
    pub transport: Transport,
    /// Over TCP, who the handler's gate found the caller to be.
    pub principal: Option<String>,
}

/// A request's transport: the Unix socket (the server's own user), or TCP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    Local,
    Tcp,
}

/// A handler's reply: status, JSON body, `Allow`, `WWW-Authenticate`, and
/// whether it ends the connection (`Connection: close`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceReply {
    pub status: u16,
    pub body: Vec<u8>,
    pub allow: Option<&'static str>,
    pub challenge: Option<&'static str>,
    pub close: bool,
}

impl ServiceReply {
    /// `value` as the body of a reply of `status` (no code is sent as 500).
    pub fn json(status: u16, value: &serde_json::Value) -> Self {
        let body = value.to_string().into_bytes();
        Self {
            status,
            body,
            allow: None,
            challenge: None,
            close: false,
        }
    }
}

/// What answers the service's requests, on a blocking thread.
pub trait ServiceHandler: Send + Sync {
    /// A TCP request's gate, given its one `Authorization`, asked before
    /// anything else of it is judged: the caller's principal.
    /// # Errors
    /// The refusal to answer with.
    fn authenticate_remote(&self, authorization: Option<&str>) -> Result<String, ServiceReply>;
    fn answer(&self, request: &ServiceRequest) -> ServiceReply;
    /// The methods `path` takes (an `Allow` value), or `None` for no route.
    fn allowed(&self, path: &str) -> Option<&'static str>;
}

/// Reads who is at the other end of an accepted connection.
pub trait PeerCredentials: Send + Sync {
    /// The peer's effective uid; a peer whose uid cannot be read is refused.
    /// # Errors
    /// The credentials could not be read.
    fn uid(&self, stream: &UnixStream) -> io::Result<u32>;
}

/// [`PeerCredentials`] by `SO_PEERCRED`, taken when the peer connected.
#[derive(Debug, Clone, Copy, Default)]
pub struct SocketPeerCredentials;

impl PeerCredentials for SocketPeerCredentials {
    fn uid(&self, stream: &UnixStream) -> io::Result<u32> {
        Ok(stream.peer_cred()?.uid())
    }
}

/// This process's effective uid: the one a server lets in.
pub fn this_uid() -> u32 {
    // SAFETY: `geteuid` has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

/// What a listener serves with: handler, peer reader, the uid let in.
pub struct ServiceParts {
    pub handler: Arc<dyn ServiceHandler>,
    pub peers: Arc<dyn PeerCredentials>,
    pub uid: u32,
    pub limits: ConnectionLimits,
}

/// Each connection's deadlines and its per-source and per-token caps;
/// tests change them.
#[derive(Debug, Clone, Copy)]
pub struct ConnectionLimits {
    pub write_timeout: Duration,
    pub lifetime: Duration,
    pub grace: Duration,
    /// How long a TCP connection has to authenticate its first request, on
    /// loopback and from elsewhere.
    pub first_request: Duration,
    pub first_request_remote: Duration,
    pub per_source: usize,
    /// The TCP connections one token may hold at once.
    pub per_token: usize,
}

impl ConnectionLimits {
    pub const PRODUCTION: Self = Self {
        write_timeout: limits::WRITE_TIMEOUT,
        lifetime: limits::CONNECTION_LIFETIME,
        grace: limits::SHUTDOWN_GRACE,
        first_request: limits::FIRST_REQUEST_TIMEOUT,
        first_request_remote: limits::FIRST_REQUEST_REMOTE_TIMEOUT,
        per_source: limits::MAX_UNPROVEN_PER_SOURCE,
        per_token: limits::MAX_CONNECTIONS_PER_TOKEN,
    };
}

/// Why a socket was not bound: a server answers on it, or why not.
#[derive(Debug)]
pub enum BindError {
    AlreadyServing(PathBuf),
    Refused(String),
}

impl std::fmt::Display for BindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyServing(path) => {
                write!(f, "a server is already serving on {}", path.display())
            }
            Self::Refused(why) => f.write_str(why),
        }
    }
}

/// A bound listener: the Unix socket, or a TCP address.
#[derive(Debug)]
pub enum Listener {
    Unix(std::os::unix::net::UnixListener),
    Tcp(std::net::TcpListener),
}

/// Binds TCP on `address` (its caller holds the allowlist): the listener
/// and the address bound.
/// # Errors
/// The bind failed.
pub fn bind_tcp(
    address: std::net::SocketAddr,
) -> Result<(std::net::TcpListener, std::net::SocketAddr), BindError> {
    let refused = |error: io::Error| BindError::Refused(format!("{address}: {error}"));
    let listener = std::net::TcpListener::bind(address).map_err(refused)?;
    let bound = listener.local_addr().map_err(refused)?;
    Ok((listener, bound))
}

/// Binds a Unix socket at `path`, mode 0600, in a directory of mode 0700
/// (made if missing). Only a stale socket of this user's is replaced.
/// # Errors
/// A relative path, a shared directory, a path holding anything but a
/// stale socket of this user's, or the bind.
pub fn bind(path: &Path) -> Result<std::os::unix::net::UnixListener, BindError> {
    let shown = path.display();
    let relative = || BindError::Refused(format!("the socket path {shown} is not absolute"));
    private_dir(
        path.parent()
            .filter(|_| path.is_absolute())
            .ok_or_else(relative)?,
    )?;
    let io = |error| BindError::Refused(format!("{shown}: {error}"));
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Ok(meta) if meta.file_type().is_socket() && meta.uid() == this_uid() => {
            match std::os::unix::net::UnixStream::connect(path) {
                Ok(_) => return Err(BindError::AlreadyServing(path.to_path_buf())),
                Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {
                    fs::remove_file(path).map_err(io)?;
                }
                Err(error) => return Err(io(error)),
            }
        }
        Ok(_) => {
            let why =
                format!("{shown} exists and is not a socket of this user's: it is left as it is");
            return Err(BindError::Refused(why));
        }
        Err(error) => return Err(io(error)),
    }
    let listener = std::os::unix::net::UnixListener::bind(path).map_err(io)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(io)?;
    match fs::symlink_metadata(path).map_err(io)?.permissions().mode() & 0o777 {
        0o600 => Ok(listener),
        _ => Err(io(io::Error::other("the socket did not take mode 0600"))),
    }
}

/// `dir` as a directory of mode 0700 of this user's, made so if missing.
fn private_dir(dir: &Path) -> Result<(), BindError> {
    let refused = |why: &str| BindError::Refused(format!("{}: {why}", dir.display()));
    let io = |error: io::Error| refused(&error.to_string());
    match fs::symlink_metadata(dir).map_err(|error| error.kind()) {
        Ok(_) => {}
        Err(io::ErrorKind::NotFound) => match fs::DirBuilder::new().mode(0o700).create(dir) {
            Ok(()) => fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).map_err(io)?,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(io(error)),
        },
        Err(kind) => return Err(io(kind.into())),
    }
    let meta = fs::symlink_metadata(dir).map_err(io)?;
    match (
        meta.is_dir(),
        meta.uid() == this_uid(),
        meta.permissions().mode() & 0o777 == 0o700,
    ) {
        (true, true, true) => Ok(()),
        (false, _, _) => Err(refused("not a directory (a symlink is not followed)")),
        (true, false, _) => Err(refused("not owned by this user")),
        (true, true, false) => Err(refused("must be mode 0700 (owner only)")),
    }
}

/// Serves `listeners` until `stop` completes, at most
/// [`limits::MAX_CONNECTIONS`] connections at once on each.
/// # Errors
/// A listener could not be handed to the runtime.
pub async fn serve(
    listeners: Vec<Listener>,
    parts: ServiceParts,
    stop: impl Future<Output = ()>,
) -> io::Result<()> {
    let accepting = listeners
        .into_iter()
        .map(Accepting::from_std)
        .collect::<io::Result<Vec<_>>>()?;
    let answers = Answers::new(limits::MAX_TCP_ANSWERS, limits::MAX_ANSWERS_PER_PRINCIPAL);
    let connections = Shares::new(parts.limits.per_token);
    let parts = Arc::new(Shared {
        parts,
        answers: Arc::new(answers),
        connections: Arc::new(connections),
    });
    let (stopping, stopped) = watch::channel(());
    let mut loops = tokio::task::JoinSet::new();
    for listener in accepting {
        let slots = Arc::new(Semaphore::new(limits::MAX_CONNECTIONS));
        loops.spawn(accept(listener, Arc::clone(&parts), slots, stopped.clone()));
    }
    stop.await;
    // Each loop ends as it sees the stop, or the sender gone.
    let _ = stopping.send(());
    drop(stopping);
    while loops.join_next().await.is_some() {}
    Ok(())
}

/// What a server's connections share: its parts; the blocking answers,
/// and the connections, TCP callers hold (across every TCP address).
struct Shared {
    parts: ServiceParts,
    answers: Arc<Answers>,
    connections: Arc<Shares>,
}

/// A listener on the runtime.
enum Accepting {
    Unix(UnixListener),
    Tcp(TcpListener),
}

/// An accepted connection, a TCP one with its place in its source's queue.
enum Accepted {
    Unix(UnixStream),
    Tcp(TcpStream, Unproven),
}

/// What a connection's requests share; `pending` is a TCP connection's
/// place until a request of it authenticates, and `held` its place among
/// its token's connections from then on (#2727).
struct Conn {
    handler: Arc<dyn ServiceHandler>,
    transport: Transport,
    pending: std::sync::Mutex<Option<Unproven>>,
    answers: Arc<Answers>,
    connections: Arc<Shares>,
    held: std::sync::Mutex<Option<(String, OwnedSemaphorePermit)>>,
}

impl Accepting {
    fn from_std(listener: Listener) -> io::Result<Self> {
        match listener {
            Listener::Unix(listener) => {
                listener.set_nonblocking(true)?;
                Ok(Self::Unix(UnixListener::from_std(listener)?))
            }
            Listener::Tcp(listener) => {
                listener.set_nonblocking(true)?;
                Ok(Self::Tcp(TcpListener::from_std(listener)?))
            }
        }
    }

    /// The next connection, a TCP one entered among its source's.
    async fn accept(&self, sources: &Sources, most: usize) -> io::Result<Accepted> {
        match self {
            Self::Unix(listener) => Ok(Accepted::Unix(listener.accept().await?.0)),
            Self::Tcp(listener) => {
                let (stream, peer) = listener.accept().await?;
                let unproven = sources.enter(source_of(peer.ip()), most);
                Ok(Accepted::Tcp(stream, unproven))
            }
        }
    }
}

/// Accepts on `listener` while a slot is free, until `stopped` changes.
async fn accept(
    listener: Accepting,
    parts: Arc<Shared>,
    slots: Arc<Semaphore>,
    mut stopped: watch::Receiver<()>,
) {
    let sources = Sources::default();
    loop {
        let slot = tokio::select! {
            _ = stopped.changed() => return,
            slot = Arc::clone(&slots).acquire_owned() => match slot {
                Ok(slot) => slot,
                // The slots are never closed; were they, nothing more is served.
                Err(_) => return,
            },
        };
        let accepted = tokio::select! {
            _ = stopped.changed() => return,
            accepted = listener.accept(&sources, parts.parts.limits.per_source) => accepted,
        };
        match accepted {
            Ok(stream) => {
                let parts = Arc::clone(&parts);
                tokio::spawn(async move {
                    admit(stream, &parts).await;
                    drop(slot);
                });
            }
            // Out of descriptors, or a peer gone before it was accepted:
            // the listener stays, and pauses so it never spins.
            Err(error) => {
                tracing::warn!(%error, "board service: accept failed");
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
}

/// A connection by its transport: a Unix peer only as the allowed uid.
async fn admit(stream: Accepted, shared: &Shared) {
    let parts = &shared.parts;
    let conn = |transport, unproven| Conn {
        handler: Arc::clone(&parts.handler),
        transport,
        pending: std::sync::Mutex::new(unproven),
        answers: Arc::clone(&shared.answers),
        connections: Arc::clone(&shared.connections),
        held: std::sync::Mutex::new(None),
    };
    match stream {
        Accepted::Unix(stream) => match parts.peers.uid(&stream) {
            Ok(uid) if uid == parts.uid => {
                connection(stream, conn(Transport::Local, None), &parts.limits).await;
            }
            Ok(_) | Err(_) => refuse(stream).await,
        },
        Accepted::Tcp(stream, unproven) => {
            let _ = stream.set_nodelay(true);
            let conn = conn(Transport::Tcp, Some(unproven));
            connection(stream, conn, &parts.limits).await;
        }
    }
}

/// One admitted connection, served request by request; a TCP one is
/// closed if it has not authenticated in its first-request time, or is
/// evicted before.
async fn connection<S>(stream: S, conn: Conn, bounds: &ConnectionLimits)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (signals, first) = match conn.pending.lock().as_deref() {
        Ok(Some(place)) if place.loopback() => {
            (Some(Arc::clone(&place.signals)), bounds.first_request)
        }
        Ok(Some(place)) => (
            Some(Arc::clone(&place.signals)),
            bounds.first_request_remote,
        ),
        Ok(None) | Err(_) => (None, bounds.first_request),
    };
    let (mut awaiting, heard) = (
        signals.is_some(),
        signals.as_ref().map(|s| Arc::clone(&s.heard)),
    );
    let signals = signals.unwrap_or_default();
    let (conn, served) = (Arc::new(conn), AtomicUsize::new(0));
    let shared = Arc::clone(&conn);
    // The last request a connection takes is answered `Connection: close`.
    let service = hyper::service::service_fn(move |request| {
        let last = served.fetch_add(1, Ordering::Relaxed) + 1 >= limits::MAX_REQUESTS;
        answer(Arc::clone(&shared), request, last)
    });
    let io = TokioIo::new(WriteDeadline::new(stream, bounds.write_timeout, heard));
    let builder = limits::http1_builder();
    let connection = builder.serve_connection(io, service);
    let unproven_end = async move {
        tokio::select! {
            () = tokio::time::sleep(first) => {}
            () = signals.evicted.notified() => {}
        }
    };
    let lifetime = tokio::time::sleep(bounds.lifetime);
    tokio::pin!(connection, unproven_end, lifetime);
    let still_unproven = || {
        conn.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    };
    // Past its lifetime a connection finishes what it is answering, then
    // closes. A peer that goes, or is too slow, ends its own connection.
    let ended = loop {
        tokio::select! {
            ended = connection.as_mut() => break ended,
            () = &mut unproven_end, if awaiting => match still_unproven() {
                true => return,
                false => awaiting = false,
            },
            () = &mut lifetime => {
                connection.as_mut().graceful_shutdown();
                // What it still answers is bounded too: past it, it is dropped.
                break tokio::time::timeout(bounds.grace, connection).await.unwrap_or(Ok(()));
            }
        }
    };
    if let Err(error) = ended {
        tracing::debug!(%error, "board service: connection ended");
    }
}

/// Answers a peer not let in with 403, parsing nothing it sent (only
/// draining it, to a bound, so it reads the answer whole).
async fn refuse(mut stream: UnixStream) {
    const BODY: &str =
        r#"{"error":"forbidden: only the server's own user may connect","code":"peer"}"#;
    let reply = format!(
        "HTTP/1.1 403 Forbidden\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\
         connection: close\r\n\r\n{BODY}",
        BODY.len()
    );
    let bound = Duration::from_secs(1);
    let _ = tokio::time::timeout(bound, stream.write_all(reply.as_bytes())).await;
    let _ = stream.shutdown().await;
    let mut sent = vec![0; limits::MAX_HEAD_BYTES];
    let _ = tokio::time::timeout(bound, stream.read(&mut sent)).await;
}

/// One request, its reply closing the connection when it is the `last`
/// the connection takes or the reply asks to.
async fn answer(
    conn: Arc<Conn>,
    request: Request<Incoming>,
    last: bool,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let reply = admitted(&conn, request).await;
    let close = last || reply.close;
    let mut reply = response(reply);
    if close {
        let close = header::HeaderValue::from_static("close");
        reply.headers_mut().insert(header::CONNECTION, close);
    }
    Ok(reply)
}

/// A request admitted by its head, its body read under its bound, then
/// answered by the handler on a blocking thread (a TCP caller holding no
/// more than its share of them).
async fn admitted(conn: &Conn, request: Request<Incoming>) -> ServiceReply {
    let path = request.uri().path().to_string();
    let (principal, room) = match conn.transport {
        Transport::Tcp => match gate(conn, request.headers()) {
            Ok((principal, room)) => (Some(principal), room),
            Err(refusal) => return refusal,
        },
        Transport::Local => (None, Room::Placed),
    };
    let handler = &conn.handler;
    // On a full token every answer ends its connection, which holds no
    // place: a refusal is answered as itself, a request read whole `busy`.
    let closing = |reply: ServiceReply| match room {
        Room::Placed => reply,
        Room::Full => ServiceReply {
            close: true,
            ..reply
        },
    };
    // hyper has parsed `Content-Length` (refusing one that is no number):
    // the body's exact size is the length it declared.
    let length = hyper::body::Body::size_hint(request.body()).exact();
    let chunked = request.headers().contains_key(header::TRANSFER_ENCODING);
    let admitted = match limits::admit(request.method().as_str(), length, chunked) {
        Ok(admitted) => admitted,
        // A method outside the allowlist: 405 naming what its path takes.
        Err(Refusal::Method) => {
            return closing(match handler.allowed(&path) {
                Some(allow) => ServiceReply {
                    allow: Some(allow),
                    ..error(405, "method", Refusal::Method.verdict().2)
                },
                None => error(404, "route", "no such route"),
            });
        }
        Err(refusal) => {
            let (status, code, why) = refusal.verdict();
            return closing(error(status, code, why));
        }
    };
    let body = match read_body(request.into_body(), admitted.body_bytes).await {
        Ok(body) => body,
        Err(reply) => return closing(reply),
    };
    // Its body read (bounded), a request on a full token is answered whole,
    // never reset by a close racing a late body.
    match room {
        Room::Placed => {}
        Room::Full => return closing(busy_connections()),
    }
    let held = match principal
        .as_deref()
        .map(|principal| conn.answers.admit(principal))
    {
        None => None,
        Some(Some(permits)) => Some(permits),
        Some(None) => {
            return error(
                503,
                "busy",
                "too many requests in flight: try again shortly",
            );
        }
    };
    let (handler, transport) = (Arc::clone(handler), conn.transport);
    let request = ServiceRequest {
        method: admitted.method,
        path,
        body,
        transport,
        principal,
    };
    tokio::task::spawn_blocking(move || (handler.answer(&request), held).0)
        .await
        .unwrap_or_else(|_| error(500, "handler", "the request's handler failed"))
}

/// The body, of `declared` bytes, read whole within the read timeout.
async fn read_body(body: Incoming, declared: u64) -> Result<Vec<u8>, ServiceReply> {
    if declared == 0 {
        return Ok(Vec::new());
    }
    let most = usize::try_from(limits::MAX_BODY_BYTES).unwrap_or(usize::MAX);
    match tokio::time::timeout(limits::READ_TIMEOUT, Limited::new(body, most).collect()).await {
        Ok(Ok(collected)) => {
            let bytes = collected.to_bytes().to_vec();
            // hyper reads the declared length; anything else is refused.
            match u64::try_from(bytes.len()) == Ok(declared) {
                true => Ok(bytes),
                false => Err(error(
                    400,
                    "length",
                    "the body is not of its Content-Length",
                )),
            }
        }
        Ok(Err(_)) => Err(error(400, "body", "the body could not be read")),
        Err(_) => Err(error(408, "timeout", "the body did not arrive in time")),
    }
}

/// Whether a request's connection holds a place among its token's
/// connections (a Unix one needs none), or its token has none free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Room {
    Placed,
    Full,
}

/// A TCP request's principal and its connection's room, or a refusal that
/// ends the connection, before anything else is judged. A placed
/// connection leaves its source's queue; one on a full token stays in it
/// (evicted, and closed past its first-request time, as unauthenticated
/// ones are), so connections waiting to be told `busy` never fill the
/// slots.
fn gate(conn: &Conn, headers: &header::HeaderMap) -> Result<(String, Room), ServiceReply> {
    let principal = conn
        .handler
        .authenticate_remote(authorization(headers).as_deref())
        .map_err(|refusal| ServiceReply {
            close: true,
            ..refusal
        })?;
    match hold(conn, &principal) {
        true => {
            drop(
                conn.pending
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take(),
            );
            Ok((principal, Room::Placed))
        }
        false => Ok((principal, Room::Full)),
    }
}

/// The refusal of a request whose token holds all its connections.
fn busy_connections() -> ServiceReply {
    error(
        503,
        "busy",
        "too many connections for this token: close one, or try again shortly",
    )
}

/// Whether `conn` holds a place among `principal`'s connections: the one
/// it holds already, else one taken now (giving back the place it held
/// for another principal). The place lives as long as the connection.
fn hold(conn: &Conn, principal: &str) -> bool {
    // The slot is replaced whole, so a poisoned lock is as good.
    let mut held = conn.held.lock().unwrap_or_else(PoisonError::into_inner);
    match held.as_ref() {
        Some((holder, _)) if holder == principal => true,
        Some(_) | None => match conn.connections.take(principal) {
            Some(place) => {
                // A connection holds one place at once: replacing the slot
                // gives back the other token's place it held, if any.
                drop(held.replace((principal.to_owned(), place)));
                true
            }
            None => false,
        },
    }
}

/// The one `Authorization`; an empty one (which no handler takes) for
/// several, or one not text.
fn authorization(headers: &header::HeaderMap) -> Option<String> {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    match (values.next(), values.next()) {
        (None, _) => None,
        (Some(value), None) => Some(value.to_str().unwrap_or_default().to_string()),
        (Some(_), Some(_)) => Some(String::new()),
    }
}

/// An error reply with its typed `code`.
fn error(status: u16, code: &str, why: &str) -> ServiceReply {
    ServiceReply::json(status, &serde_json::json!({ "error": why, "code": code }))
}

fn response(reply: ServiceReply) -> Response<Full<Bytes>> {
    let status = StatusCode::from_u16(reply.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut response = Response::new(Full::new(Bytes::from(reply.body)));
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

#[cfg(test)]
#[path = "listener_tests.rs"]
mod tests;
