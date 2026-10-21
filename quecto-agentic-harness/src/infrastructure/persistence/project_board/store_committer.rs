//! Who committed a board commit, read back exactly as recorded; and the
//! board's remote, by its URL and by the id its records name.
use super::*;
use crate::domain::project_board::value_objects::telemetry::BoardId;
use sha2::{Digest, Sha256};

impl GitBoardStore {
    /// The committer of `commit`, exactly as recorded, fetching the board
    /// first when this clone does not hold it yet.
    ///
    /// # Errors
    /// The commit is not on the board, or git fails.
    pub fn committed_by(&self, commit: &str) -> Result<Identity, BoardStoreError> {
        let commit = object_id(commit)?;
        let args = [
            "log",
            "-1",
            "--no-show-signature",
            "--format=%cn%x00%ce",
            commit.as_str(),
        ];
        let bytes = match self.git.ok(&args, None) {
            Ok(bytes) => bytes,
            Err(GitError::Failed { .. }) => {
                self.fetch()?;
                self.git.ok(&args, None).map_err(unavailable)?
            }
            Err(error) => return Err(unavailable(error)),
        };
        let text = String::from_utf8_lossy(&bytes);
        let (name, email) = text
            .trim_end_matches('\n')
            .split_once('\0')
            .ok_or_else(|| BoardStoreError::Unavailable(format!("no committer in {commit}")))?;
        Ok(Identity {
            name: name.into(),
            email: email.into(),
        })
    }

    /// The remote's fetch URL, as git resolves it (`insteadOf` applied).
    ///
    /// # Errors
    /// No such remote, or git fails.
    pub fn remote_url(&self) -> Result<String, BoardStoreError> {
        self.git
            .text(&["remote", "get-url", "--", self.remote.as_str()], None)
            .map_err(unavailable)
    }

    /// The id a board's records name it by (#2663): the first 64 bits of
    /// the SHA-256 of its remote's host and path ([`repository_of`]), as
    /// 16 hex digits, so an https, ssh or `.git` spelling of one repository
    /// gives one id, and a token in the URL never changes it. The URL
    /// itself is never recorded.
    pub fn board_id_of(url: &str) -> Option<BoardId> {
        let digest = Sha256::digest(repository_of(url).as_bytes());
        let hex: String = digest[..8]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        BoardId::of_hex(&hex)
    }
}

/// The repository a remote's fetch `url` names, as its board id is made
/// from it: its host in lower case and its path, the scheme and any
/// userinfo left out, an scp-style `user@host:path` read as host and path,
/// and a trailing `/` and `.git` dropped. A local path is its path, so
/// trimmed.
pub(super) fn repository_of(url: &str) -> String {
    let (host, path) = match url.split_once("://") {
        Some((_, rest)) => match rest.split_once('/') {
            Some((authority, path)) => (authority, format!("/{path}")),
            None => (rest, String::new()),
        },
        // `user@host:path`: a `:` before any `/`.
        None => match url.split_once(':') {
            Some((authority, path)) if !authority.contains('/') => {
                (authority, format!("/{}", path.trim_start_matches('/')))
            }
            Some(_) | None => ("", url.to_string()),
        },
    };
    let host = host.rsplit_once('@').map_or(host, |(_, host)| host);
    let path = path.trim_end_matches('/');
    let path = path
        .strip_suffix(".git")
        .unwrap_or(path)
        .trim_end_matches('/');
    format!("{}{path}", host.to_ascii_lowercase())
}
