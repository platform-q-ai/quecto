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
        // A repository git cannot use (not one, or one it distrusts as
        // another user's) fails here with its own cause, rather than
        // reading as one with no committer configured.
        asking.ok(&["rev-parse", "--git-dir"], None)?;
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

    /// [`Self::reader`], with git never asked whether it would record the
    /// placeholder as given (#2705 B1): a reader commits nothing, so
    /// building it starts no process. For a front end that knows where its
    /// board is; a repository git cannot run in then fails at the first
    /// read rather than here.
    ///
    /// # Errors
    /// No git on the `PATH`.
    pub fn reader_unasked(repo: &Path, env: GitEnvironment) -> Result<Self, GitError> {
        let program = env
            .get("PATH")
            .and_then(|path| on_path(path, "git"))
            .ok_or(GitError::NotFound)?;
        assert!(program.is_absolute(), "git is run by its absolute path");
        let (name, email) = READER;
        let identity = Identity {
            name: name.into(),
            email: email.into(),
        };
        debug_assert!(identity.validate("identity").is_ok());
        Ok(Self {
            identity,
            ..Self::unidentified(program, repo, &env)
        })
    }

    /// This git, every call bounded by `bound` (local or reaching the
    /// remote): for a read that must answer soon, as a teardown's (#2666).
    #[must_use]
    pub fn bounded(mut self, bound: std::time::Duration) -> Self {
        self.local_timeout = self.local_timeout.min(bound);
        self.network_timeout = self.network_timeout.min(bound);
        self
    }
}

/// What `git config --get` answered.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ConfigAnswer {
    /// The value it resolves.
    Set(String),
    /// Nothing sets it: git's own answer for that is exit status 1 and no
    /// message.
    Unset,
    /// Anything else: git failed to read its configuration.
    Failed,
}

/// The answer an exit `code` (`None` for a signal), `stdout` and `stderr`
/// of `git config --get` mean.
fn config_answer(code: Option<i32>, stdout: &[u8], stderr: &[u8]) -> ConfigAnswer {
    match (code, stderr.is_empty()) {
        (Some(0), _) => ConfigAnswer::Set(String::from_utf8_lossy(stdout).trim_end().to_string()),
        (Some(1), true) => ConfigAnswer::Unset,
        (Some(_) | None, _) => ConfigAnswer::Failed,
    }
}

/// The value `git config --get key` resolves, or `None` when nothing sets
/// it. Any other failure (a timeout, a broken configuration) is git's
/// error, with its cause.
fn config_get(git: &Git, key: &str) -> Result<Option<String>, GitError> {
    let args = ["config", "--get", key];
    let output = git.local(&args, None, SHORT_OUTPUT)?;
    match config_answer(output.status.code(), &output.stdout, &output.stderr) {
        ConfigAnswer::Set(value) => Ok(Some(value)),
        ConfigAnswer::Unset => Ok(None),
        ConfigAnswer::Failed => Err(failed(&args, &output)),
    }
}

#[cfg(test)]
#[path = "git_committer_tests.rs"]
mod tests;
