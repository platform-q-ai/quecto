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
        let asking = Self::unidentified(program, repo, env);
        let configured = |key: &str| match config_get(&asking, key)? {
            Some(value) => Ok(value),
            None => Err(GitError::Identity(format!(
                "{key} is not configured: set it to write the board"
            ))),
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

    /// git for `repo` with no identity of its own, for asking its
    /// configuration: `git config` reads none of the identity variables
    /// `run` sets.
    fn unidentified(program: PathBuf, repo: &Path, env: &GitEnvironment) -> Self {
        Self {
            program,
            repo: repo.to_path_buf(),
            env: env.clone(),
            identity: Identity {
                name: String::new(),
                email: String::new(),
            },
            local_timeout: LOCAL_TIMEOUT,
            network_timeout: NETWORK_TIMEOUT,
        }
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

/// The value `git config --get key` resolves, or `None` when nothing sets
/// it: git's own answer for that is exit status 1 and no message. Any
/// other failure (no repository there, a timeout, a broken configuration)
/// is git's error, with its cause.
fn config_get(git: &Git, key: &str) -> Result<Option<String>, GitError> {
    let args = ["config", "--get", key];
    let output = git.local(&args, None, SHORT_OUTPUT)?;
    match (output.status.code(), output.stderr.is_empty()) {
        (Some(0), _) => Ok(Some(
            String::from_utf8_lossy(&output.stdout)
                .trim_end()
                .to_string(),
        )),
        (Some(1), true) => Ok(None),
        (Some(_) | None, _) => Err(failed(&args, &output)),
    }
}
