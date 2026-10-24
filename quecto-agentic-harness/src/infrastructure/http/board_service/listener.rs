//! The board service's listener (epic #2705 B4): HTTP/1.1 on a Unix
//! socket, parsed by hyper; it knows no board. A connection is let in only
//! when the injected [`PeerCredentials`] read the one uid allowed; each
//! request is held to the [`limits`] and handed to the injected
//! [`ServiceHandler`] on a blocking thread. [`bind`] makes the socket
//! safely: mode 0600, in a directory of mode 0700.
use std::convert::Infallible;
use std::fs;
use std::future::Future;
use std::io;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use http_body_util::{BodyExt, Full, Limited};
use hyper::body::{Bytes, Incoming};
use hyper::{Request, Response, StatusCode, header};
use hyper_util::rt::TokioIo;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Semaphore;

use super::limits::{self, Refusal, WriteDeadline};

pub use super::limits::Method as ServiceMethod;

/// A request as the handler gets it: admitted, its body read whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceRequest {
    pub method: ServiceMethod,
    /// The request target's path, without its query.
    pub path: String,
    pub body: Vec<u8>,
}

/// A handler's reply: a status, a JSON body, and a 405's `Allow`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceReply {
    pub status: u16,
    pub body: Vec<u8>,
    pub allow: Option<&'static str>,
}

impl ServiceReply {
    /// `value` as the body of a reply of `status` (no code is sent as 500).
    pub fn json(status: u16, value: &serde_json::Value) -> Self {
        let body = value.to_string().into_bytes();
        Self {
            status,
            body,
            allow: None,
        }
    }
}

/// What answers the service's requests, on a blocking thread.
pub trait ServiceHandler: Send + Sync {
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

/// Each connection's deadlines; tests shorten them.
#[derive(Debug, Clone, Copy)]
pub struct ConnectionLimits {
    pub write_timeout: Duration,
    pub lifetime: Duration,
}

impl ConnectionLimits {
    pub const PRODUCTION: Self = Self {
        write_timeout: limits::WRITE_TIMEOUT,
        lifetime: limits::CONNECTION_LIFETIME,
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

/// Serves `listener` until `stop` completes, at most
/// [`limits::MAX_CONNECTIONS`] connections at once.
/// # Errors
/// The listener could not be handed to the runtime.
pub async fn serve(
    listener: std::os::unix::net::UnixListener,
    parts: ServiceParts,
    stop: impl Future<Output = ()>,
) -> io::Result<()> {
    listener.set_nonblocking(true)?;
    let listener = UnixListener::from_std(listener)?;
    let slots = Arc::new(Semaphore::new(limits::MAX_CONNECTIONS));
    let parts = Arc::new(parts);
    tokio::pin!(stop);
    loop {
        let slot = tokio::select! {
            () = &mut stop => return Ok(()),
            slot = Arc::clone(&slots).acquire_owned() => match slot {
                Ok(slot) => slot,
                // The slots are never closed; were they, nothing more is served.
                Err(_) => return Ok(()),
            },
        };
        let accepted = tokio::select! {
            () = &mut stop => return Ok(()),
            accepted = listener.accept() => accepted,
        };
        match accepted {
            Ok((stream, _)) => {
                let parts = Arc::clone(&parts);
                tokio::spawn(async move {
                    connection(stream, &parts).await;
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

/// One connection: refused unless its peer is the allowed uid, else
/// served request by request.
async fn connection(stream: UnixStream, parts: &ServiceParts) {
    match parts.peers.uid(&stream) {
        Ok(uid) if uid == parts.uid => {}
        Ok(_) | Err(_) => return refuse(stream).await,
    }
    let (handler, served) = (Arc::clone(&parts.handler), AtomicUsize::new(0));
    // The last request a connection takes is answered `Connection: close`.
    let service = hyper::service::service_fn(move |request| {
        let last = served.fetch_add(1, Ordering::Relaxed) + 1 >= limits::MAX_REQUESTS;
        answer(Arc::clone(&handler), request, last)
    });
    let io = TokioIo::new(WriteDeadline::new(stream, parts.limits.write_timeout));
    let builder = limits::http1_builder();
    let connection = builder.serve_connection(io, service);
    tokio::pin!(connection);
    // Past its lifetime a connection finishes what it is answering, then
    // closes. A peer that goes, or is too slow, ends its own connection.
    let ended = tokio::select! {
        ended = connection.as_mut() => ended,
        () = tokio::time::sleep(parts.limits.lifetime) => {
            connection.as_mut().graceful_shutdown();
            connection.await
        }
    };
    if let Err(error) = ended {
        tracing::debug!(%error, "board service: connection ended");
    }
}

/// Answers a peer not let in with 403, parsing nothing it sent (only
/// draining it, to a bound, so it reads the answer whole).
async fn refuse(mut stream: UnixStream) {
    const BODY: &str = r#"{"error":"forbidden: only the server's own user may connect"}"#;
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
/// the connection takes.
async fn answer(
    handler: Arc<dyn ServiceHandler>,
    request: Request<Incoming>,
    last: bool,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let mut reply = response(admitted(handler, request).await);
    if last {
        let close = header::HeaderValue::from_static("close");
        reply.headers_mut().insert(header::CONNECTION, close);
    }
    Ok(reply)
}

/// A request admitted by its head, its body read under its bound, then
/// answered by the handler on a blocking thread.
async fn admitted(handler: Arc<dyn ServiceHandler>, request: Request<Incoming>) -> ServiceReply {
    let path = request.uri().path().to_string();
    let headers = request.headers();
    let lengths: Vec<&[u8]> = headers
        .get_all(header::CONTENT_LENGTH)
        .iter()
        .map(|value| value.as_bytes())
        .collect();
    let chunked = headers.contains_key(header::TRANSFER_ENCODING);
    let admitted = match limits::admit(request.method().as_str(), &lengths, chunked) {
        Ok(admitted) => admitted,
        // A method outside the allowlist: 405 naming what its path takes.
        Err(Refusal::Method) => {
            return match handler.allowed(&path) {
                Some(allow) => ServiceReply {
                    allow: Some(allow),
                    ..error(405, Refusal::Method.verdict().1)
                },
                None => error(404, "no such route"),
            };
        }
        Err(refusal) => return error(refusal.verdict().0, refusal.verdict().1),
    };
    let body = match read_body(request.into_body(), admitted.body_bytes).await {
        Ok(body) => body,
        Err(reply) => return reply,
    };
    let request = ServiceRequest {
        method: admitted.method,
        path,
        body,
    };
    tokio::task::spawn_blocking(move || handler.answer(&request))
        .await
        .unwrap_or_else(|_| error(500, "the request's handler failed"))
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
                false => Err(error(400, "the body is not of its Content-Length")),
            }
        }
        Ok(Err(_)) => Err(error(400, "the body could not be read")),
        Err(_) => Err(error(408, "the body did not arrive in time")),
    }
}

fn error(status: u16, why: &str) -> ServiceReply {
    ServiceReply::json(status, &serde_json::json!({ "error": why }))
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
    response
}

#[cfg(test)]
#[path = "listener_tests.rs"]
mod tests;
