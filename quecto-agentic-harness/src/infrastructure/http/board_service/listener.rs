//! Red stub (B4): the listener's surface, not yet served.
use std::future::Future;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceMethod {
    Get,
    Post,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceRequest {
    pub method: ServiceMethod,
    pub path: String,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceReply {
    pub status: u16,
    pub body: Vec<u8>,
}

impl ServiceReply {
    pub fn json(status: u16, value: &serde_json::Value) -> Self {
        Self {
            status,
            body: value.to_string().into_bytes(),
        }
    }
}

pub trait ServiceHandler: Send + Sync {
    fn answer(&self, request: &ServiceRequest) -> ServiceReply;
}

pub trait PeerCredentials: Send + Sync {
    /// # Errors
    /// The peer's credentials could not be read.
    fn uid(&self, stream: &tokio::net::UnixStream) -> std::io::Result<u32>;
}

pub struct SocketPeerCredentials;

impl PeerCredentials for SocketPeerCredentials {
    fn uid(&self, _stream: &tokio::net::UnixStream) -> std::io::Result<u32> {
        Err(std::io::Error::other("not yet read"))
    }
}

pub fn this_uid() -> u32 {
    0
}

pub struct ServiceParts {
    pub handler: Arc<dyn ServiceHandler>,
    pub peers: Arc<dyn PeerCredentials>,
    pub uid: u32,
}

#[derive(Debug)]
pub struct BindError(pub String);

impl std::fmt::Display for BindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// # Errors
/// Not yet bound.
pub fn bind(path: &Path) -> Result<std::os::unix::net::UnixListener, BindError> {
    Err(BindError(format!("{}: not yet served", path.display())))
}

/// # Errors
/// Not yet served.
pub async fn serve(
    listener: std::os::unix::net::UnixListener,
    parts: ServiceParts,
    stop: impl Future<Output = ()>,
) -> std::io::Result<()> {
    drop((listener, parts));
    stop.await;
    Ok(())
}

#[cfg(test)]
#[path = "listener_tests.rs"]
mod tests;
