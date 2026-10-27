//! Git for tests, isolated from the machine running them
//! (swarm-container-matches-repo review): a test's `git init` or
//! `remote add` must never reach a real repository, whatever a hook or a
//! shell exported, nor read the user's or the system's configuration.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// `git -C <dir> <args>` with every `GIT_*` variable of this process
/// removed (`GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE`,
/// `GIT_OBJECT_DIRECTORY`, `GIT_ALTERNATE_OBJECT_DIRECTORIES`,
/// `GIT_CONFIG_PARAMETERS`, `GIT_CONFIG_COUNT`/`KEY_*`/`VALUE_*`, …),
/// `HOME` and `XDG_CONFIG_HOME` at `home`, and no system configuration.
pub fn isolated_git(home: &Path, dir: &Path, args: &[&str]) -> Output {
    let mut command = Command::new("git");
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("GIT_") {
            command.env_remove(&name);
        }
    }
    command
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .output()
        .expect("git runs")
}

/// [`isolated_git`], asserting it succeeded.
pub fn isolated_git_ok(home: &Path, dir: &Path, args: &[&str]) {
    let output = isolated_git(home, dir, args);
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A new checkout at `<root>/project` whose `origin` is `origin`, made by
/// [`isolated_git`] with `root` as its home.
pub fn checkout_with_origin(root: &Path, origin: &str) -> PathBuf {
    let project = root.join("project");
    std::fs::create_dir_all(&project).expect("project directory");
    isolated_git_ok(root, &project, &["init", "-q"]);
    isolated_git_ok(root, &project, &["remote", "add", "origin", origin]);
    project
}

#[cfg(test)]
#[path = "isolated_git_tests.rs"]
mod tests;
