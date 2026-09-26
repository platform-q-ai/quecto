use super::*;

fn only(paths: &'static [&'static str]) -> impl Fn(&str) -> bool {
    move |path| paths.contains(&path)
}

/// #2195: with `SHELL` unset (a container), bash runs the command where it
/// is installed, not `/bin/sh`.
#[test]
fn bash_runs_when_shell_is_unset() {
    assert_eq!(
        select_shell(None, only(&["/usr/bin/bash"])),
        "/usr/bin/bash"
    );
    assert_eq!(
        select_shell(None, only(&["/bin/bash", "/usr/bin/bash"])),
        "/bin/bash"
    );
}

/// A plain POSIX shell in `SHELL` gives way to bash too: the tool is bash.
#[test]
fn bash_is_preferred_over_a_requested_posix_shell() {
    for requested in ["/bin/sh", "/bin/dash"] {
        assert_eq!(
            select_shell(Some(requested), only(&["/bin/bash"])),
            "/bin/bash"
        );
    }
}

/// An allowed bash or zsh in `SHELL` is kept as the user's choice.
#[test]
fn a_requested_bash_or_zsh_is_kept() {
    assert_eq!(
        select_shell(Some("/usr/bin/zsh"), only(&["/bin/bash"])),
        "/usr/bin/zsh"
    );
    assert_eq!(
        select_shell(Some("/usr/local/bin/bash"), only(&["/bin/bash"])),
        "/usr/local/bin/bash"
    );
}

/// Without bash the requested allowed shell, else `/bin/sh`, runs it; a
/// shell outside the allowlist is never used.
#[test]
fn without_bash_the_fallbacks_hold() {
    assert_eq!(select_shell(Some("/bin/dash"), only(&[])), "/bin/dash");
    assert_eq!(select_shell(None, only(&[])), "/bin/sh");
    assert_eq!(select_shell(Some("/tmp/evil"), only(&[])), "/bin/sh");
    assert_eq!(
        select_shell(Some("/tmp/evil"), only(&["/bin/bash"])),
        "/bin/bash"
    );
}
