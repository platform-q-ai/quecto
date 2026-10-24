//! The board service's listener (epic #2705 B4): HTTP/1.1 on a Unix
//! socket, parsed by hyper. It knows no board. Each connection is let in
//! only when its peer's uid, read by the injected [`PeerCredentials`], is
//! the one uid allowed; each request is held to the [`limits`] and then
//! handed, whole, to the injected [`ServiceHandler`] on a blocking thread
//! (a board's store blocks), whose reply is sent back as JSON. Its writes
//! fail once the peer takes no byte for the write timeout, so a peer that
//! never reads frees its connection slot.
//!
//! [`bind`] makes the socket safely: in a directory of mode 0700 (made so,
//! or found so and owned by this user), never replacing a path that is
//! not a socket, nor a socket a server still answers on; the socket
//! itself is mode 0600.
use std::convert::Infallible;
use std::fs;
use std::future::Future;
use std::io;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
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

/// A handler's reply: a status and a JSON body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceReply {
    pub status: u16,
    pub body: Vec<u8>,
}

impl ServiceReply {
    /// `value` as the body of a reply of `status` (a status code, 100 to
    /// 599; any other is sent as 500).
    pub fn json(status: u16, value: &serde_json::Value) -> Self {
        // Never a panic on the data path: a code out of range is a fault.
        let status = match (100..=599).contains(&status) {
            true => status,
            false => 500,
        };
        Self {
            status,
            body: value.to_string().into_bytes(),
        }
    }
}

/// What answers the service's requests. Blocking: the listener calls it
/// on a blocking thread.
pub trait ServiceHandler: Send + Sync {
    fn answer(&self, request: &ServiceRequest) -> ServiceReply;
}

/// Reads who is at the other end of an accepted connection.
pub trait PeerCredentials: Send + Sync {
    /// The peer's effective uid.
    ///
    /// # Errors
    /// The credentials could not be read; the peer is then refused.
    fn uid(&self, stream: &UnixStream) -> io::Result<u32>;
}

/// [`PeerCredentials`] by the kernel's record of the peer
/// (`SO_PEERCRED`, taken when it connected).
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

/// What a listener serves with: the handler, the peer reader, and the one
/// uid let in.
pub struct ServiceParts {
    pub handler: Arc<dyn ServiceHandler>,
    pub peers: Arc<dyn PeerCredentials>,
    pub uid: u32,
    /// How long a write may wait for the peer to take any byte
    /// ([`limits::WRITE_TIMEOUT`] in production).
    pub write_timeout: Duration,
}

/// Why a socket was not bound.
#[derive(Debug)]
pub enum BindError {
    /// The socket's path is not absolute.
    Relative(PathBuf),
    /// Its directory is not one only this user can enter.
    Directory(PathBuf, String),
    /// Something other than this user's socket is at its path.
    NotASocket(PathBuf),
    /// A server answers on the socket already.
    AlreadyServing(PathBuf),
    Io(PathBuf, io::Error),
}

impl std::fmt::Display for BindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Relative(path) => write!(f, "the socket path {} is not absolute", path.display()),
            Self::Directory(path, why) => write!(f, "{}: {why}", path.display()),
            Self::NotASocket(path) => write!(
                f,
                "{} exists and is not a socket of this user's: it is left as it is",
                path.display()
            ),
            Self::AlreadyServing(path) => {
                write!(f, "a server is already serving on {}", path.display())
            }
            Self::Io(path, error) => write!(f, "{}: {error}", path.display()),
        }
    }
}

impl std::error::Error for BindError {}

/// Binds a Unix socket at `path`, mode 0600, in a directory of mode 0700
/// (made if missing; its parent must exist). A stale socket of this
/// user's (no server answers on it) is replaced; nothing else is.
///
/// # Errors
/// A relative path, a directory others may enter or not this user's, a
/// path holding anything but a stale socket of this user's, or the bind.
pub fn bind(path: &Path) -> Result<std::os::unix::net::UnixListener, BindError> {
    let dir = match (path.is_absolute(), path.parent()) {
        (true, Some(dir)) => dir,
        (false, _) | (true, None) => return Err(BindError::Relative(path.to_path_buf())),
    };
    private_dir(dir)?;
    let io = |error| BindError::Io(path.to_path_buf(), error);
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
        Ok(_) => return Err(BindError::NotASocket(path.to_path_buf())),
        Err(error) => return Err(io(error)),
    }
    let listener = std::os::unix::net::UnixListener::bind(path).map_err(io)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(io)?;
    let mode = fs::symlink_metadata(path).map_err(io)?.permissions().mode();
    if mode & 0o777 != 0o600 {
        return Err(io(io::Error::other("the socket did not take mode 0600")));
    }
    Ok(listener)
}

/// `dir` as a directory of mode 0700 owned by this user: made so when
/// missing, else taken only when it is so.
fn private_dir(dir: &Path) -> Result<(), BindError> {
    let refused = |why: &str| BindError::Directory(dir.to_path_buf(), why.to_string());
    let io = |error| BindError::Io(dir.to_path_buf(), error);
    if let Err(error) = fs::symlink_metadata(dir) {
        if error.kind() != io::ErrorKind::NotFound {
            return Err(io(error));
        }
        match fs::DirBuilder::new().mode(0o700).create(dir) {
            Ok(()) => fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).map_err(io)?,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(io(error)),
        }
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

/// Serves `listener` until `stop` completes: each connection on its own
/// task, at most [`limits::MAX_CONNECTIONS`] at once.
///
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
    let handler = Arc::clone(&parts.handler);
    let service = hyper::service::service_fn(move |request| answer(Arc::clone(&handler), request));
    // A peer that goes, or is too slow, ends its own connection only.
    if let Err(error) = limits::http1_builder()
        .serve_connection(
            TokioIo::new(WriteDeadline::new(stream, parts.write_timeout)),
            service,
        )
        .await
    {
        tracing::debug!(%error, "board service: connection ended");
    }
}

/// Answers a peer that is not let in with 403, without parsing anything
/// it sent; what it sent is read and dropped (to a bound) so it reads the
/// answer whole.
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

/// One request: admitted by its head, its body read under its bound, then
/// answered by the handler on a blocking thread.
async fn answer(
    handler: Arc<dyn ServiceHandler>,
    request: Request<Incoming>,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let headers = request.headers();
    let lengths: Vec<&[u8]> = headers
        .get_all(header::CONTENT_LENGTH)
        .iter()
        .map(|value| value.as_bytes())
        .collect();
    let chunked = headers.contains_key(header::TRANSFER_ENCODING);
    let admitted = match limits::admit(request.method().as_str(), &lengths, chunked) {
        Ok(admitted) => admitted,
        Err(refusal) => return Ok(refused(refusal)),
    };
    let path = request.uri().path().to_string();
    let body = match read_body(request.into_body(), admitted.body_bytes).await {
        Ok(body) => body,
        Err(reply) => return Ok(response(reply)),
    };
    let request = ServiceRequest {
        method: admitted.method,
        path,
        body,
    };
    let reply = tokio::task::spawn_blocking(move || handler.answer(&request))
        .await
        .unwrap_or_else(|_| error(500, "the request's handler failed"));
    Ok(response(reply))
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

fn refused(refusal: Refusal) -> Response<Full<Bytes>> {
    let mut reply = response(error(refusal.status(), refusal.reason()));
    if refusal == Refusal::Method {
        reply
            .headers_mut()
            .insert(header::ALLOW, header::HeaderValue::from_static("GET, POST"));
    }
    reply
}

fn response(reply: ServiceReply) -> Response<Full<Bytes>> {
    let status = StatusCode::from_u16(reply.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    let mut response = Response::new(Full::new(Bytes::from(reply.body)));
    *response.status_mut() = status;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("application/json"),
    );
    response
}

#[cfg(test)]
#[path = "listener_tests.rs"]
mod tests;
