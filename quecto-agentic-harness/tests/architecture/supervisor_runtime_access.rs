//! Who may reach the supervisor's runtime (#2286).

use super::{RUNTIME_SPAWNERS, SUPERVISOR, supervisor_fns};

/// Every way `files` reach the supervisor's runtime outside the
/// allowlisted helpers, and every stale allowlist entry.
pub(super) fn violations(files: &[(String, syn::File)]) -> Vec<String> {
    let mut found = Vec::new();
    let mut used = Vec::new();
    for function in supervisor_fns(files).iter().filter(|f| f.touches_handle) {
        let allowed = RUNTIME_SPAWNERS
            .iter()
            .find(|(name, _, _)| *name == function.name)
            .filter(|_| function.file == SUPERVISOR);
        let Some((name, visibility, _)) = allowed else {
            found.push(format!(
                "{}: `{}` touches the supervisor's runtime handle",
                function.file, function.name
            ));
            continue;
        };
        if function.visibility != *visibility {
            found.push(format!(
                "{SUPERVISOR}: `{name}` changed its pinned visibility"
            ));
        }
        used.push(*name);
    }
    for (name, _, _) in RUNTIME_SPAWNERS {
        if !used.contains(name) {
            found.push(format!(
                "RUNTIME_SPAWNERS lists `{name}`, which reaches no runtime: remove it"
            ));
        }
    }
    found
}
