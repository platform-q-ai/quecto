//! A swarm launch's repository check (swarm-container-matches-repo): what
//! the launching project's repository and the container config's are, and
//! the refusal of a launch whose container is not confirmed to clone the
//! project's own repository. The refusal's text is agent-facing and
//! user-visible: it names both repositories and how to set this
//! repository's container up.

use std::fmt;
use std::path::PathBuf;

use crate::domain::environments::entities::repository_identity::RepositoryIdentity;
use crate::domain::shared::redaction::model_safe_text;

/// The repository a container config's create command clones, as launch
/// policy read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerRepository {
    /// `--repo <url>` naming this repository.
    Named(RepositoryIdentity),
    /// `--repo <url>` with a URL that names no repository by host, owner
    /// and name (a local path, a `file://` URL, …); the URL as written.
    Unreadable(String),
    /// No `--repo` at all: a sandbox, or a script that clones some other
    /// way. Which repository it holds cannot be told.
    Unnamed,
    /// `--repo` more than once, each URL as written: which one the script
    /// clones cannot be told.
    Repeated(Vec<String>),
}

/// The launching project's repository, as launch policy read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectRepository {
    /// The checkout's `origin` names this repository.
    Known {
        checkout: PathBuf,
        identity: RepositoryIdentity,
    },
    /// The checkout's `origin` is a URL naming no repository by host,
    /// owner and name; the URL as written.
    Unreadable { checkout: PathBuf, origin: String },
    /// Not a git checkout, or one without `origin`.
    NoOrigin { checkout: PathBuf },
    /// Git could not be asked; why.
    OriginUnavailable { checkout: PathBuf, reason: String },
    /// The launcher was composed with no project checkout.
    NoCheckout,
}

/// A swarm launch refused because its container config is not confirmed
/// to clone the launching project's own repository: the two do not match,
/// or either cannot be determined. Raised before anything is created. Only
/// [`Self::new`] makes one, so a matching pair is never a refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerRepositoryRefusal {
    /// The selected container config's name.
    config: String,
    container: ContainerRepository,
    project: ProjectRepository,
}

/// The opening every refusal shares.
pub const REFUSAL_OPENING: &str = "swarm launch refused before anything was created";

/// A URL as a refusal may show it: userinfo (a token in an `https` URL, a
/// password in git's scp form) and secret shapes redacted, bounded.
fn shown_url(url: &str) -> String {
    let url = match (url.contains("://"), url.split_once('@')) {
        (false, Some((_, rest))) => format!("***@{rest}"),
        (true, _) | (false, None) => url.to_string(),
    };
    model_safe_text(&url)
}

impl ContainerRepositoryRefusal {
    /// The refusal of a swarm launch from `project` into container config
    /// `config`, which clones `container`; `None` when the two are known
    /// and the same repository, which is no refusal.
    pub fn new(
        config: impl Into<String>,
        container: ContainerRepository,
        project: ProjectRepository,
    ) -> Option<Self> {
        match (&project, &container) {
            (ProjectRepository::Known { identity, .. }, ContainerRepository::Named(cloned))
                if identity == cloned =>
            {
                None
            }
            _ => Some(Self {
                config: config.into(),
                container,
                project,
            }),
        }
    }

    /// The selected container config's name.
    pub fn config(&self) -> &str {
        &self.config
    }

    /// The repository the container config clones, as read.
    pub fn container(&self) -> &ContainerRepository {
        &self.container
    }

    /// The launching project's repository, as read.
    pub fn project(&self) -> &ProjectRepository {
        &self.project
    }

    fn project_clause(&self) -> String {
        match &self.project {
            ProjectRepository::Known { checkout, identity } => format!(
                "this project ({}) is repository {identity}",
                checkout.display()
            ),
            ProjectRepository::Unreadable { checkout, origin } => format!(
                "the origin remote of this project ({}) is {}, which quecto cannot read as a repository",
                checkout.display(),
                shown_url(origin)
            ),
            ProjectRepository::NoOrigin { checkout } => format!(
                "this project ({}) is not a git checkout with an origin remote, so its repository is unknown",
                checkout.display()
            ),
            ProjectRepository::OriginUnavailable { checkout, reason } => format!(
                "the origin remote of this project ({}) could not be read ({})",
                checkout.display(),
                model_safe_text(reason)
            ),
            ProjectRepository::NoCheckout => {
                "this launcher knows no project checkout, so the project's repository is unknown"
                    .to_string()
            }
        }
    }

    fn container_clause(&self) -> String {
        let config = &self.config;
        match &self.container {
            ContainerRepository::Named(identity) => {
                format!("container config '{config}' clones {identity}")
            }
            ContainerRepository::Unreadable(url) => format!(
                "container config '{config}' clones {}, which quecto cannot read as a repository",
                shown_url(url)
            ),
            ContainerRepository::Unnamed => format!(
                "container config '{config}' names no repository (its create command has no --repo)"
            ),
            ContainerRepository::Repeated(urls) => format!(
                "container config '{config}' names --repo more than once ({}), so which repository it clones cannot be told",
                urls.iter()
                    .map(|url| shown_url(url))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }

    fn advice(&self) -> String {
        match (&self.project, &self.container) {
            (
                ProjectRepository::Known { checkout, identity },
                ContainerRepository::Named(container),
            ) => {
                debug_assert_ne!(identity, container, "Self::new never refuses a match");
                format!(
                    "A swarm runs only in a container of its own repository: set this repository's container up with `quecto container init --project {} --repo {}`, then start again.",
                    checkout.display(),
                    identity.https_url()
                )
            }
            (
                ProjectRepository::Known { checkout, identity },
                ContainerRepository::Unreadable(_)
                | ContainerRepository::Unnamed
                | ContainerRepository::Repeated(_),
            ) => format!(
                "quecto cannot confirm that container clones this repository: set this repository's container up with `quecto container init --project {} --repo {}`, then start again.",
                checkout.display(),
                identity.https_url()
            ),
            (
                ProjectRepository::Unreadable { .. }
                | ProjectRepository::NoOrigin { .. }
                | ProjectRepository::OriginUnavailable { .. }
                | ProjectRepository::NoCheckout,
                _,
            ) => "quecto cannot confirm that container clones the project's repository: start the swarm from a clone whose origin is its repository, with that repository's container set up by `quecto container init --project <dir> --repo <url>`.".to_string(),
        }
    }
}

impl fmt::Display for ContainerRepositoryRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{REFUSAL_OPENING}: {}, but {}. {}",
            self.project_clause(),
            self.container_clause(),
            self.advice()
        )
    }
}

impl std::error::Error for ContainerRepositoryRefusal {}
