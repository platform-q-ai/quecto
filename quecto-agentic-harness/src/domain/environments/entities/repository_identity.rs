//! The repository a git remote names, as launch policy compares two of
//! them (swarm-container-matches-repo): its host, owner and name, each in
//! lower case. GitHub treats owner and repository names
//! case-insensitively, and so does this comparison; the host is compared
//! too, so the same owner and name on two hosts are two repositories.
//! Reading one out of a URL (`https`, `ssh`, scp form, `.git`, trailing
//! slashes) is infrastructure's: this value only holds parts already
//! split, each of an allowed shape.

use std::fmt;

/// The longest part an identity holds, in bytes.
pub const MAX_PART_BYTES: usize = 256;

/// The host whose identities read as `owner/name` alone.
pub const GITHUB_HOST: &str = "github.com";

/// One repository, by host, owner and name, each lower case.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepositoryIdentity {
    host: String,
    owner: String,
    name: String,
}

impl RepositoryIdentity {
    /// The repository `host`/`owner`/`name` names, its parts lower-cased;
    /// `None` when a part is not of the allowed shape: a host name of
    /// ASCII letters, digits, `.` and `-`, starting with a letter or digit
    /// and holding a `.` (no port, no bare `localhost` or drive letter); an owner and a name of ASCII letters, digits, `.`, `_` and
    /// `-`, not starting with `-` and neither `.` nor `..`; each at most
    /// [`MAX_PART_BYTES`].
    pub fn new(host: &str, owner: &str, name: &str) -> Option<Self> {
        match (is_host(host), is_segment(owner), is_segment(name)) {
            (true, true, true) => Some(Self {
                host: host.to_ascii_lowercase(),
                owner: owner.to_ascii_lowercase(),
                name: name.to_ascii_lowercase(),
            }),
            _ => None,
        }
    }

    /// The repository's `https` URL, with no credential in it: what a
    /// container of this repository is set up with.
    pub fn https_url(&self) -> String {
        format!("https://{}/{}/{}", self.host, self.owner, self.name)
    }
}

/// `owner/name` on GitHub, `host/owner/name` elsewhere.
impl fmt::Display for RepositoryIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.host == GITHUB_HOST {
            true => write!(f, "{}/{}", self.owner, self.name),
            false => write!(f, "{}/{}/{}", self.host, self.owner, self.name),
        }
    }
}

fn is_host(host: &str) -> bool {
    let first = host
        .bytes()
        .next()
        .is_some_and(|b| b.is_ascii_alphanumeric());
    first
        && host.contains('.')
        && host.len() <= MAX_PART_BYTES
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b))
}

fn is_segment(segment: &str) -> bool {
    let first = segment
        .bytes()
        .next()
        .is_some_and(|b| b.is_ascii_alphanumeric() || b"._".contains(&b));
    first
        && segment.len() <= MAX_PART_BYTES
        && !matches!(segment, "." | "..")
        && segment
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

#[cfg(test)]
#[path = "repository_identity_tests.rs"]
mod tests;
