//! A swarm's container clones the launching project's own repository
//! (swarm-container-matches-repo). A project with no container config of
//! its own falls back to the global default, and a global default names
//! one repository: launching a swarm into it from any other project put
//! the swarm in a clone of the wrong repository. This use case is the
//! rule: a swarm launches only when the container config's repository
//! (its create command's `--repo`) and the project's (its checkout's
//! `origin`) are both known and the same repository by host, owner and
//! name (case-insensitive). Anything else is refused before anything is
//! created, conservatively: a config naming no repository, or one either
//! side of which cannot be read, is refused as unconfirmable, never
//! assumed to match.
//!
//! Blocking: reading the project's origin runs git on the calling thread.

use std::path::PathBuf;
use std::sync::Arc;

use crate::application::subagents::dto::container_config::{
    ConfigRepository, ContainerLaunchConfig,
};
use crate::application::subagents::dto::container_repository::{
    ContainerRepository, ContainerRepositoryRefusal, ProjectRepository,
};
use crate::application::subagents::ports::container_repository::{ProjectOrigin, RepositoryUrls};
use crate::domain::environments::entities::repository_identity::RepositoryIdentity;

pub struct ConfirmContainerRepository {
    /// The launching project's checkout; `None` for a launcher composed
    /// without one, which confirms nothing.
    checkout: Option<PathBuf>,
    origins: Arc<dyn ProjectOrigin>,
    urls: Arc<dyn RepositoryUrls>,
}

impl ConfirmContainerRepository {
    pub fn new(
        checkout: Option<PathBuf>,
        origins: Arc<dyn ProjectOrigin>,
        urls: Arc<dyn RepositoryUrls>,
    ) -> Self {
        Self {
            checkout,
            origins,
            urls,
        }
    }

    /// The repository `config` clones, when it is confirmed to be the
    /// project's own.
    ///
    /// # Errors
    /// The two repositories differ, or either cannot be determined: the
    /// refusal names both as far as they are known.
    pub fn confirm(
        &self,
        config: &ContainerLaunchConfig,
    ) -> Result<RepositoryIdentity, Box<ContainerRepositoryRefusal>> {
        let project = self.project();
        let container = self.container(config);
        let confirmed = match (&project, &container) {
            (ProjectRepository::Known { identity, .. }, ContainerRepository::Named(cloned))
                if identity == cloned =>
            {
                Some(identity.clone())
            }
            _ => None,
        };
        match ContainerRepositoryRefusal::new(config.name.clone(), container, project) {
            None => Ok(confirmed.expect("only a confirmed pair is no refusal")),
            Some(refusal) => Err(Box::new(refusal)),
        }
    }

    fn project(&self) -> ProjectRepository {
        let Some(checkout) = self.checkout.clone() else {
            return ProjectRepository::NoCheckout;
        };
        match self.origins.origin(&checkout) {
            Ok(Some(origin)) => match self.urls.identity(&origin) {
                Some(identity) => ProjectRepository::Known {
                    checkout,
                    origin,
                    identity,
                },
                None => ProjectRepository::Unreadable { checkout, origin },
            },
            Ok(None) => ProjectRepository::NoOrigin { checkout },
            Err(reason) => ProjectRepository::OriginUnavailable { checkout, reason },
        }
    }

    fn container(&self, config: &ContainerLaunchConfig) -> ContainerRepository {
        match &config.repository {
            ConfigRepository::Named(url) => match self.urls.identity(url) {
                Some(identity) => ContainerRepository::Named(identity),
                None => ContainerRepository::Unreadable(url.clone()),
            },
            ConfigRepository::Repeated(urls) => ContainerRepository::Repeated(urls.clone()),
            ConfigRepository::Unnamed => ContainerRepository::Unnamed,
        }
    }
}

impl std::fmt::Debug for ConfirmContainerRepository {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfirmContainerRepository")
            .field("checkout", &self.checkout)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[path = "confirm_container_repository_tests.rs"]
mod tests;
