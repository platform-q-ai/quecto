//! Which shell runs a `bash` tool command (#2195). The tool is named and
//! described as bash, so bash runs it wherever bash is installed, even when
//! `SHELL` is unset or names a plain POSIX shell (as in a container, where
//! `/bin/sh` is dash and bash syntax would silently misbehave).

/// Shells that may be selected via the `SHELL` environment variable.
///
/// Restricted to well-known system shells to prevent arbitrary binary execution
/// via a crafted or injected `SHELL` env var.
pub(super) const ALLOWED_SHELLS: &[&str] = &[
    "/bin/sh",
    "/bin/bash",
    "/bin/dash",
    "/bin/zsh",
    "/usr/bin/bash",
    "/usr/bin/zsh",
    "/usr/local/bin/bash",
    "/usr/local/bin/zsh",
];

/// Requested shells kept as they are: bash only. zsh differs from bash in
/// word splitting, arrays and globbing, and expands `=cmd` in ways the
/// command policy's bash model does not see (#2195 review), so an installed
/// bash wins over it too.
const KEPT_AS_REQUESTED: &[&str] = &["/bin/bash", "/usr/bin/bash", "/usr/local/bin/bash"];

/// Where bash is looked for, in order, when the requested shell is not kept.
const BASH_LOCATIONS: &[&str] = &["/bin/bash", "/usr/bin/bash", "/usr/local/bin/bash"];

/// The shell for a command: the requested one when it is an allowed bash;
/// otherwise the first installed bash; otherwise the requested allowed
/// shell; otherwise `/bin/sh`. `installed` says whether a path exists.
pub(super) fn select_shell(
    requested: Option<&str>,
    installed: impl Fn(&str) -> bool,
) -> &'static str {
    let allowed = requested.and_then(|shell| ALLOWED_SHELLS.iter().copied().find(|s| *s == shell));
    // A requested bash that is not there is no choice at all (#2195 review).
    if let Some(kept) =
        allowed.filter(|shell| KEPT_AS_REQUESTED.contains(shell) && installed(shell))
    {
        return kept;
    }
    BASH_LOCATIONS
        .iter()
        .copied()
        .find(|path| installed(path))
        .or(allowed)
        .unwrap_or("/bin/sh")
}

/// Whether a shell path exists as a file (a symlink counts by its target).
pub(super) fn is_installed(path: &str) -> bool {
    std::path::Path::new(path).is_file()
}

/// Whether commands run under bash here, or a plain POSIX shell.
pub(super) fn runs_bash(shell: &str) -> bool {
    KEPT_AS_REQUESTED.contains(&shell)
}

/// The shell for this process's commands, chosen once: the tool's
/// description and every command then agree (#2195 review).
pub(super) fn default_shell() -> &'static str {
    static CHOSEN: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
    CHOSEN.get_or_init(|| select_shell(std::env::var("SHELL").ok().as_deref(), is_installed))
}

#[cfg(test)]
#[path = "shell_tests.rs"]
mod tests;
