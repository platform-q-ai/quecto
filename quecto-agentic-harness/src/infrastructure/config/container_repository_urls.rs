//! Reads a git remote URL as the repository it names
//! (swarm-container-matches-repo): the subagent capability's
//! [`RepositoryUrls`] port. The forms git itself takes for a hosted
//! repository are read — `https`/`http`, `ssh`, `git`, `git+ssh`/`ssh+git`
//! URLs and git's scp form `[user@]host:owner/name` — with any userinfo
//! and port dropped, trailing slashes and one `.git` suffix dropped, and
//! the parts lower-cased by the domain's [`RepositoryIdentity`]. Anything
//! else (a local path, a `file://` URL, a path deeper or shallower than
//! `owner/name`, a query or fragment) names no repository here, and launch
//! policy refuses what it cannot confirm.

use crate::application::subagents::ports::container_repository::RepositoryUrls;
use crate::domain::environments::entities::repository_identity::RepositoryIdentity;

/// The URL schemes a hosted repository is cloned over.
const SCHEMES: [&str; 6] = ["https", "http", "ssh", "git", "git+ssh", "ssh+git"];

#[derive(Debug, Default, Clone, Copy)]
pub struct RemoteRepositoryUrls;

impl RepositoryUrls for RemoteRepositoryUrls {
    fn identity(&self, url: &str) -> Option<RepositoryIdentity> {
        repository_identity(url)
    }
}

/// The repository `url` names, as the module says; `None` when it names
/// none by host, owner and name.
pub fn repository_identity(url: &str) -> Option<RepositoryIdentity> {
    let url = url.trim();
    let (authority, path) = match url.split_once("://") {
        Some((scheme, rest)) => {
            let scheme = scheme.to_ascii_lowercase();
            match SCHEMES.contains(&scheme.as_str()) {
                true => rest.split_once('/')?,
                false => return None,
            }
        }
        // git's scp form: a `:` before any `/`, and a host before it.
        None => match url.split_once(':') {
            Some((authority, path)) if !authority.contains('/') => (authority, path),
            Some(_) | None => return None,
        },
    };
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = match host.split_once(':') {
        Some((host, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => host,
        Some(_) => return None,
        None => host,
    };
    let path = path.trim_matches('/');
    let path = match path.len().checked_sub(".git".len()) {
        Some(cut) if path.is_char_boundary(cut) && path[cut..].eq_ignore_ascii_case(".git") => {
            &path[..cut]
        }
        Some(_) | None => path,
    };
    match path.split('/').collect::<Vec<_>>().as_slice() {
        [owner, name] => RepositoryIdentity::new(host, owner, name),
        _ => None,
    }
}

#[cfg(test)]
#[path = "container_repository_urls_tests.rs"]
mod tests;
