//! What swarm launch policy reads to confirm a container clones the
//! launching project's own repository (swarm-container-matches-repo).

use std::path::Path;

use crate::domain::environments::entities::repository_identity::RepositoryIdentity;

/// The launching project's `origin` remote. Implemented by infrastructure
/// over git (`git -C <checkout> remote get-url origin`, local
/// configuration only, no network); composition binds the checkout.
pub trait ProjectOrigin: Send + Sync {
    /// The `origin` URL of the checkout `checkout` lies in; `Ok(None)`
    /// when it is not a git checkout or has no `origin`; `Err` carries why
    /// git could not be asked.
    fn origin(&self, checkout: &Path) -> Result<Option<String>, String>;
}

/// Reads a remote URL as the repository it names. Implemented by
/// infrastructure, which owns the URL forms (`https`, `ssh`, git's scp
/// form, `.git`, trailing slashes, userinfo and ports).
pub trait RepositoryUrls: Send + Sync {
    /// The repository `url` names; `None` when it is not a URL of a
    /// repository by host, owner and name (a local path, a `file://` URL,
    /// a nested group path, anything else).
    fn identity(&self, url: &str) -> Option<RepositoryIdentity>;
}
