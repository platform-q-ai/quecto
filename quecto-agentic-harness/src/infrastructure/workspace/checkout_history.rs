//! The shared checkout's commit history (#2687): `CheckoutHistory` over
//! `git merge-base --is-ancestor`, run in the board's checkout.
//!
//! git runs with an allowlisted environment (so a `GIT_DIR` or
//! `GIT_WORK_TREE` of this process cannot point it at another repository),
//! no terminal prompt, and a deadline. Only full commit ids reach it, so no
//! argument can read as an option or a revision expression. Every answer
//! but git's own "yes" (exit 0) is `false`.
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::application::swarm::ports::board_files::CheckoutHistory;
use crate::domain::swarm::services::evidence_carry::full_commit_id;

/// The only variables git inherits: where to find it, and the user's own
/// configuration (where a container's `safe.directory` lives).
const INHERITED: &[&str] = &[
    "PATH",
    "HOME",
    "XDG_CONFIG_HOME",
    "GIT_CONFIG_GLOBAL",
    "GIT_CONFIG_NOSYSTEM",
    "GIT_CEILING_DIRECTORIES",
];
/// How long one ancestry question may take: a local walk of the commit
/// graph, so far longer than any healthy answer.
const DEADLINE: Duration = Duration::from_secs(10);
/// The longest pause between looks at whether git exited.
const POLL: Duration = Duration::from_millis(5);

/// git's history of the checkout at `root`.
#[derive(Clone, Debug)]
pub struct GitCheckoutHistory {
    root: PathBuf,
}

impl GitCheckoutHistory {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn command(&self, ancestor: &str, descendant: &str) -> Command {
        let mut command = Command::new("git");
        command
            .env_clear()
            .envs(
                INHERITED.iter().filter_map(|name| {
                    std::env::var_os(name).map(|value| (name.to_owned(), value))
                }),
            )
            .env("GIT_TERMINAL_PROMPT", "0")
            .arg("-C")
            .arg(&self.root)
            .args(["merge-base", "--is-ancestor", ancestor, descendant])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }
}

impl CheckoutHistory for GitCheckoutHistory {
    fn is_ancestor(&self, ancestor: &str, descendant: &str) -> bool {
        let (true, true) = (full_commit_id(ancestor), full_commit_id(descendant)) else {
            return false;
        };
        let Ok(mut child) = self.command(ancestor, descendant).spawn() else {
            return false;
        };
        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return status.code() == Some(0),
                Ok(None) if started.elapsed() < DEADLINE => thread::sleep(POLL),
                Ok(None) | Err(_) => {
                    // Out of time, or unwaitable: end it and answer no.
                    let _ = child.kill();
                    let _ = child.wait();
                    return false;
                }
            }
        }
    }
}
