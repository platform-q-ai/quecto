//! The shared checkout's commit history (#2687): `CheckoutHistory` over
//! `git merge-base --is-ancestor`, run in the board's checkout.
//!
//! git runs with an allowlisted environment (so a `GIT_DIR` or
//! `GIT_WORK_TREE` of this process cannot point it at another repository),
//! no terminal prompt, and one deadline for the whole question. Only full
//! commit ids reach it, so no argument can read as an option or a revision
//! expression. Every answer but git's own "yes" (exit 0) for every commit
//! asked about declines the carry, with its reason.
use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::application::swarm::ports::board_files::{Ancestry, CheckoutHistory};
use crate::domain::swarm::services::evidence_carry::full_commit_id;

/// The variables git inherits by name: where to find it, and the user's
/// own configuration (where a container's `safe.directory` lives).
const INHERITED: &[&str] = &[
    "PATH",
    "HOME",
    "XDG_CONFIG_HOME",
    "GIT_CONFIG_GLOBAL",
    "GIT_CONFIG_NOSYSTEM",
    "GIT_CEILING_DIRECTORIES",
    "GIT_CONFIG_COUNT",
];
/// The variable families git inherits by prefix: command-scope
/// configuration (`GIT_CONFIG_KEY_<n>`, `GIT_CONFIG_VALUE_<n>`, counted by
/// `GIT_CONFIG_COUNT`), which a container may use for `safe.directory`. It
/// sets configuration only; it never points git at another repository.
const INHERITED_PREFIXES: &[&str] = &["GIT_CONFIG_KEY_", "GIT_CONFIG_VALUE_"];
/// How long one whole ancestry question may take: local walks of the
/// commit graph, so far longer than any healthy answer.
const DEADLINE: Duration = Duration::from_secs(10);
/// The longest pause between looks at whether git exited.
const POLL: Duration = Duration::from_millis(5);

/// Whether git inherits the variable `name`.
fn inherited(name: &OsStr) -> bool {
    name.to_str().is_some_and(|name| {
        INHERITED.contains(&name)
            || INHERITED_PREFIXES
                .iter()
                .any(|prefix| name.starts_with(prefix))
    })
}

/// git's history of the checkout at `root`.
#[derive(Clone, Debug)]
pub struct GitCheckoutHistory {
    root: PathBuf,
    environment: Vec<(OsString, OsString)>,
    deadline: Duration,
}

impl GitCheckoutHistory {
    /// The history at `root`, git inheriting this process's environment
    /// as it is now, allowlisted, under the default deadline.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self::with_environment(root, std::env::vars_os(), DEADLINE)
    }

    /// The history at `root`, git inheriting `environment`, allowlisted,
    /// and answering each question within `deadline`.
    pub fn with_environment(
        root: impl Into<PathBuf>,
        environment: impl IntoIterator<Item = (OsString, OsString)>,
        deadline: Duration,
    ) -> Self {
        Self {
            root: root.into(),
            environment: environment
                .into_iter()
                .filter(|(name, _)| inherited(name))
                .collect(),
            deadline,
        }
    }

    fn command(&self, ancestor: &str, descendant: &str) -> Command {
        let mut command = Command::new("git");
        command
            .env_clear()
            .envs(self.environment.iter().cloned())
            .env("GIT_TERMINAL_PROMPT", "0")
            .arg("-C")
            .arg(&self.root)
            .args(["merge-base", "--is-ancestor", ancestor, descendant])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    /// One `merge-base --is-ancestor`, ended at `deadline`.
    fn ask(&self, ancestor: &str, descendant: &str, deadline: Instant) -> Ancestry {
        let Ok(mut child) = self.command(ancestor, descendant).spawn() else {
            return Ancestry::GitFailed;
        };
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    return match status.code() {
                        Some(0) => Ancestry::Confirmed,
                        Some(1) => Ancestry::NotAncestor,
                        _ => Ancestry::GitFailed,
                    };
                }
                Ok(None) if Instant::now() < deadline => thread::sleep(POLL),
                waited => {
                    // Out of time, or unwaitable: end it, reap it, and
                    // answer why.
                    let _ = child.kill();
                    let _ = child.wait();
                    return match waited {
                        Ok(_) => Ancestry::TimedOut,
                        Err(_) => Ancestry::GitFailed,
                    };
                }
            }
        }
    }
}

impl CheckoutHistory for GitCheckoutHistory {
    fn ancestry(&self, ancestors: &[String], descendant: &str) -> Ancestry {
        let answer = self.answer(ancestors, descendant);
        if answer != Ancestry::Confirmed {
            tracing::info!(
                checkout = %self.root.display(),
                descendant,
                reason = answer.as_str(),
                "swarm evidence carry declined"
            );
        }
        answer
    }
}

impl GitCheckoutHistory {
    /// [`CheckoutHistory::ancestry`]'s answer, untraced.
    fn answer(&self, ancestors: &[String], descendant: &str) -> Ancestry {
        let full = !ancestors.is_empty()
            && full_commit_id(descendant)
            && ancestors.iter().all(|id| full_commit_id(id));
        if !full {
            return Ancestry::NotFullId;
        }
        let deadline = Instant::now() + self.deadline;
        for ancestor in ancestors {
            match self.ask(ancestor, descendant, deadline) {
                Ancestry::Confirmed => {}
                declined => return declined,
            }
        }
        Ancestry::Confirmed
    }
}
