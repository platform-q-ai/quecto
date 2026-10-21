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
    /// the SHA-256 of its remote's host and path ([`repository_of`], a
    /// local remote resolved from `repo`, the clone's top-level directory, as
    /// git resolves it), as 16 hex
    /// digits, so an https, ssh or `.git` spelling of one repository gives
    /// one id, and a token in the URL never changes it. The URL itself is
    /// never recorded. The id is opaque, not secret: anyone who knows a
    /// repository can compute its id.
    pub fn board_id_of(url: &str, repo: &std::path::Path) -> Option<BoardId> {
        let digest = Sha256::digest(repository_of(url, repo).as_bytes());
        let hex: String = digest[..8]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        BoardId::of_hex(&hex)
    }
}

/// The repository a remote's fetch `url` names, as its board id is made
/// from it: its host in lower case (its scheme's default port left out:
/// ssh 22, http 80, https 443, git 9418) and its path (its case kept),
/// the scheme and any userinfo left out, an scp-style `user@host:path`
/// read as ssh's host and path, and a trailing `/` and `.git` dropped. A
/// local remote (a path, or a `file://` URL) is the canonical absolute path
/// it names from `repo`, the clone's top-level directory, so trimmed.
pub(super) fn repository_of(url: &str, repo: &std::path::Path) -> String {
    let (scheme, host, path) = match url.split_once("://") {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("file") => {
            ("file", "", local_path(rest, repo))
        }
        Some((scheme, rest)) => match rest.split_once('/') {
            Some((authority, path)) => (scheme, authority, format!("/{path}")),
            None => (scheme, rest, String::new()),
        },
        // `user@host:path`: a `:` before any `/`, ssh's.
        None => match url.split_once(':') {
            Some((authority, path)) if !authority.contains('/') => (
                "ssh",
                authority,
                format!("/{}", path.trim_start_matches('/')),
            ),
            Some(_) | None => ("file", "", local_path(url, repo)),
        },
    };
    let host = host.rsplit_once('@').map_or(host, |(_, host)| host);
    let default_port = match scheme.to_ascii_lowercase().as_str() {
        "ssh" | "git+ssh" | "ssh+git" => Some(":22"),
        "http" => Some(":80"),
        "https" => Some(":443"),
        "git" => Some(":9418"),
        _ => None,
    };
    let host = default_port
        .and_then(|port| host.strip_suffix(port))
        .unwrap_or(host);
    let path = path.trim_end_matches('/');
    let path = path
        .strip_suffix(".git")
        .unwrap_or(path)
        .trim_end_matches('/');
    format!("{}{path}", host.to_ascii_lowercase())
}

/// A local remote's path as an absolute one, resolved from `repo` (the
/// clone's directory) when relative, and canonical when it exists.
fn local_path(path: &str, repo: &std::path::Path) -> String {
    let joined = repo.join(path);
    let resolved = std::fs::canonicalize(&joined).unwrap_or(joined);
    resolved.to_string_lossy().into_owned()
}

impl GitBoardStore {
    /// The clone's top-level directory, which git resolves a relative
    /// remote from (#2663): its work tree's, or a bare clone's git
    /// directory's.
    ///
    /// # Errors
    /// git cannot name it.
    pub fn top_level(&self) -> Result<std::path::PathBuf, BoardStoreError> {
        let top = self
            .git
            .text(&["rev-parse", "--show-toplevel"], None)
            .or_else(|_| self.git.text(&["rev-parse", "--absolute-git-dir"], None))
            .map_err(unavailable)?;
        Ok(std::path::PathBuf::from(top))
    }
}
