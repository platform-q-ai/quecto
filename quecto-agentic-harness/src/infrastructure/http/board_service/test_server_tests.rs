//! A board service listener on a temp socket, for the transport's tests:
//! a handler that counts its calls and peers read by an injected uid.
use super::listener::{
    ConnectionLimits, PeerCredentials, ServiceHandler, ServiceParts, ServiceReply, ServiceRequest,
    bind, serve,
};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream as StdStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The uid the test server allows; peers are told apart by the injected
/// reader, never by the test's real uid.
pub const SERVER_UID: u32 = 4242;

/// Answers every request with its path and body length, counting calls;
/// each reply carries `pad` bytes of padding.
#[derive(Default)]
pub struct Counting {
    pub calls: AtomicUsize,
    pub pad: usize,
}

impl ServiceHandler for Counting {
    fn answer(&self, request: &ServiceRequest) -> ServiceReply {
        self.calls.fetch_add(1, Ordering::SeqCst);
        ServiceReply::json(
            200,
            &serde_json::json!({
                "path": request.path,
                "body": request.body.len(),
                "pad": "x".repeat(self.pad),
            }),
        )
    }

    /// `/v1/thing` takes GET and POST; no other path is a route.
    fn allowed(&self, path: &str) -> Option<&'static str> {
        (path == "/v1/thing").then_some("GET, POST")
    }
}

/// A peer-credential reader that reads every peer as its uid.
struct FixedUid(u32);

impl PeerCredentials for FixedUid {
    fn uid(&self, _stream: &tokio::net::UnixStream) -> std::io::Result<u32> {
        Ok(self.0)
    }
}

/// A listener serving on its own thread until dropped.
pub struct Running {
    _dir: tempfile::TempDir,
    pub socket: std::path::PathBuf,
    pub handler: Arc<Counting>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.stop.take().map(|stop| stop.send(()));
        let _ = self.thread.take().map(std::thread::JoinHandle::join);
    }
}

/// A listener whose peers all read as `peer_uid`.
pub fn start(peer_uid: u32) -> Running {
    start_with(peer_uid, 0, ConnectionLimits::PRODUCTION)
}

/// A listener whose peers all read as `peer_uid`, its replies padded by
/// `pad` bytes, its connections held to `limits`.
pub fn start_with(peer_uid: u32, pad: usize, limits: ConnectionLimits) -> Running {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("board").join("board.sock");
    let listener = bind(&socket).unwrap();
    let handler = Arc::new(Counting {
        calls: AtomicUsize::new(0),
        pad,
    });
    let parts = ServiceParts {
        handler: Arc::clone(&handler) as Arc<dyn ServiceHandler>,
        peers: Arc::new(FixedUid(peer_uid)),
        uid: SERVER_UID,
        limits,
    };
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let thread = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime
            .block_on(serve(listener, parts, async {
                let _ = stopped.await;
            }))
            .unwrap();
    });
    Running {
        _dir: dir,
        socket,
        handler,
        stop: Some(stop),
        thread: Some(thread),
    }
}

/// Sends `request` whole and reads the answer to its end: its status and
/// body.
pub fn raw(socket: &std::path::Path, request: &[u8]) -> (u16, String) {
    let text = whole(socket, request);
    let status = text
        .split(' ')
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or_else(|| panic!("no status line in {text:?}"));
    let body = text
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .unwrap_or_default();
    (status, body)
}

/// Sends `request` whole and reads everything the server sends until it
/// closes (or 10 s pass).
pub fn whole(socket: &std::path::Path, request: &[u8]) -> String {
    let mut stream = StdStream::connect(socket).unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(10)))
        .unwrap();
    // A server that answers before reading the whole body may close
    // first; the answer is what is judged.
    let _ = stream.write_all(request);
    let mut answer = Vec::new();
    let _ = stream.read_to_end(&mut answer);
    String::from_utf8_lossy(&answer).to_string()
}

/// A request with `headers` header lines in all (`Host` and `Connection`
/// among them) and `body`.
pub fn request(method: &str, headers: usize, extra: &str, body: &[u8]) -> Vec<u8> {
    assert!(headers >= 2, "Host and Connection are two of them");
    let mut text = format!("{method} /v1/thing HTTP/1.1\r\nHost: board\r\nConnection: close\r\n");
    let fillers = headers - 2 - extra.lines().count();
    (0..fillers).for_each(|n| text.push_str(&format!("X-Filler-{n}: {n}\r\n")));
    text.push_str(extra);
    text.push_str("\r\n");
    let mut bytes = text.into_bytes();
    bytes.extend_from_slice(body);
    bytes
}
