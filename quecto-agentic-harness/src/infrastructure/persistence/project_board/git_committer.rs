//! The committer a repository's configuration names: who the board writes
//! as. Reads need none: [`Git::reader`] reads as a placeholder identity
//! that never writes.
use super::*;

/// The identity a read-only store is built with: it never commits, since
/// the front ends build it only for requests that do not write.
pub const READER: (&str, &str) = ("quecto board reader", "reader@quecto.invalid");

impl Git {
    /// The committer `repo`'s configuration names (`user.name` and
    /// `user.email`, as `git config` resolves them). [`Self::new`] then
    /// checks git records it as given.
    ///
    /// # Errors
    /// No git on the `PATH`, no committer configured, or an identity the
    /// board cannot name.
    pub fn configured_committer(repo: &Path, env: &GitEnvironment) -> Result<Identity, GitError> {
        let program = env
            .get("PATH")
            .and_then(|path| on_path(path, "git"))
            .ok_or(GitError::NotFound)?;
        // Asked of git with no identity of its own: `git config` reads
        // none of the identity variables `run` sets.
        let asking = Self {
            program,
            repo: repo.to_path_buf(),
            env: env.clone(),
            identity: Identity {
                name: String::new(),
                email: String::new(),
            },
            local_timeout: LOCAL_TIMEOUT,
            network_timeout: NETWORK_TIMEOUT,
        };
        let configured = |key: &str| {
            asking.text(&["config", "--get", key], None).map_err(|_| {
                GitError::Identity(format!(
                    "{key} is not configured: set it to write the board"
                ))
            })
        };
        let identity = Identity {
            name: configured("user.name")?,
            email: configured("user.email")?,
        };
        identity
            .validate("committer")
            .map_err(|error| GitError::Identity(error.to_string()))?;
        Ok(identity)
    }

    /// git for `repo` that only reads: it carries the placeholder
    /// [`READER`] identity, so it is built whether or not a committer is
    /// configured.
    ///
    /// # Errors
    /// As [`Self::new`]'s.
    pub fn reader(repo: &Path, env: GitEnvironment) -> Result<Self, GitError> {
        let (name, email) = READER;
        Self::new(
            repo,
            env,
            Identity {
                name: name.into(),
                email: email.into(),
            },
        )
    }
}
