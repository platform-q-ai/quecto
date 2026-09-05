// `rm-protected-dir` (#1620): recursive deletion of directories that are never
// a legitimate build/cache target — the user's home, the agent workspace,
// any top-level system directory, and any sibling home under `/home` or
// `/Users`. Locations are resolved at check time on the host that will run
// the command, so nothing platform-specific is compiled in.

use std::path::{Component, Path, PathBuf};

/// Host facts the protected-directory rule needs. Built by the sandbox at
/// check time; tests construct it explicitly.
#[derive(Debug, Clone, Default)]
pub(crate) struct HostContext {
    /// The user's home directory (`$HOME`, `~`).
    pub home: Option<PathBuf>,
    /// The agent workspace root.
    pub workspace: Option<PathBuf>,
    /// Directory relative targets resolve against (the bash tool's cwd).
    pub cwd: Option<PathBuf>,
}

impl HostContext {
    /// Context for the current process: home from the environment, cwd and
    /// workspace both the sandbox workspace (the bash tool runs there).
    pub(crate) fn from_host(workspace: Option<&Path>) -> Self {
        Self {
            home: dirs::home_dir(),
            workspace: workspace.map(Path::to_path_buf),
            cwd: workspace.map(Path::to_path_buf),
        }
    }
}

/// Does `target`, as written in an `rm -r` argv (quotes removed, `$HOME` and
/// `~` still textual), name a protected directory?
pub(crate) fn is_protected_target(target: &str, host: &HostContext) -> bool {
    let Some(path) = resolve(target, host) else {
        return false;
    };
    let depth = path
        .components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .count();
    if depth == 0 {
        // `/` itself is rm-root's job.
        return false;
    }
    if depth == 1 {
        // `/etc`, `/usr`, `/System`, `/Users`, `/home`, … on any Unix.
        return true;
    }
    if let Some(home) = &host.home
        && lexical(home) == path
    {
        return true;
    }
    if let Some(ws) = &host.workspace
        && lexical(ws) == path
    {
        return true;
    }
    // Another user's home: exactly one level under the conventional roots.
    if depth == 2 {
        let mut comps = path.components().filter_map(|c| match c {
            Component::Normal(s) => Some(s),
            _ => None,
        });
        let root = comps.next();
        return matches!(root.and_then(|s| s.to_str()), Some("home" | "Users"));
    }
    false
}

/// Turn the textual target into an absolute, lexically normalised path.
/// Returns `None` when it cannot be anchored (relative with unknown cwd,
/// `~` with unknown home).
fn resolve(target: &str, host: &HostContext) -> Option<PathBuf> {
    // A trailing `*` or `.*` deletes the directory's contents, which for
    // protection purposes is the directory.
    let trimmed = strip_content_globs(target);
    let anchored: PathBuf = if trimmed == "~" || trimmed.starts_with("~/") {
        host.home
            .as_ref()?
            .join(trimmed.trim_start_matches('~').trim_start_matches('/'))
    } else if trimmed.starts_with('/') {
        PathBuf::from(trimmed)
    } else if trimmed.is_empty() {
        return None;
    } else {
        host.cwd.as_ref()?.join(trimmed)
    };
    Some(lexical(&anchored))
}

fn strip_content_globs(target: &str) -> &str {
    let mut t = target;
    loop {
        let next = t
            .strip_suffix("/*")
            .or_else(|| t.strip_suffix("/.*"))
            .or_else(|| t.strip_suffix("/."))
            .unwrap_or(t);
        let next = if next.len() > 1 {
            next.trim_end_matches('/')
        } else {
            next
        };
        if next == t {
            return t;
        }
        t = next;
    }
}

/// Lexical normalisation: drop `.`, resolve `..`, no filesystem access.
fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        PathBuf::from("/")
    } else {
        out
    }
}
