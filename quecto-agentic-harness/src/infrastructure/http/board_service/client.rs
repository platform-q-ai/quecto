//! Red stub (B4): the blocking client's surface.
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientReply {
    pub status: u16,
    pub body: Vec<u8>,
}

#[derive(Debug)]
pub enum ClientError {
    Connect(std::io::Error),
    Request(String),
    Io(std::io::Error),
    Malformed(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}

pub struct BoardServiceClient {
    socket: PathBuf,
}

impl BoardServiceClient {
    pub fn unix(socket: &Path) -> Self {
        Self {
            socket: socket.to_path_buf(),
        }
    }

    /// # Errors
    /// Not yet sent.
    pub fn get(&self, path: &str) -> Result<ClientReply, ClientError> {
        Err(ClientError::Malformed(format!(
            "{} {path}: not yet sent",
            self.socket.display()
        )))
    }
}

/// # Errors
/// Not yet parsed.
pub fn parse_reply(bytes: &[u8]) -> Result<ClientReply, ClientError> {
    Err(ClientError::Malformed(format!("{} bytes", bytes.len())))
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
