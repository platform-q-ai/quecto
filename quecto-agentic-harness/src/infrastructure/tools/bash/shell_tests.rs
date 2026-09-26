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

/// An allowed bash in `SHELL` is kept; zsh gives way to an installed bash
/// (#2195 review), and runs only when there is no bash.
#[test]
fn a_requested_bash_is_kept_and_zsh_gives_way_to_bash() {
    assert_eq!(
        select_shell(Some("/usr/bin/zsh"), only(&["/bin/bash"])),
        "/bin/bash"
    );
    assert_eq!(
        select_shell(Some("/usr/bin/zsh"), only(&[])),
        "/usr/bin/zsh"
    );
    assert_eq!(
        select_shell(
            Some("/usr/local/bin/bash"),
            only(&["/usr/local/bin/bash", "/bin/bash"])
        ),
        "/usr/local/bin/bash"
    );
    // A requested bash that is not installed gives way to one that is.
    assert_eq!(
        select_shell(Some("/usr/local/bin/bash"), only(&["/bin/bash"])),
        "/bin/bash"
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

/// Missing paths, directories and dangling symlinks are not installed.
#[test]
fn only_an_existing_file_is_installed() {
    let tmp = tempfile::TempDir::new().unwrap();
    let file = tmp.path().join("sh");
    std::fs::write(&file, "").unwrap();
    let dangling = tmp.path().join("dangling");
    std::os::unix::fs::symlink(tmp.path().join("nowhere"), &dangling).unwrap();
    assert!(is_installed(file.to_str().unwrap()));
    assert!(!is_installed(tmp.path().to_str().unwrap()));
    assert!(!is_installed(dangling.to_str().unwrap()));
    assert!(!is_installed(tmp.path().join("missing").to_str().unwrap()));
}

#[test]
fn only_bash_counts_as_running_bash() {
    assert!(runs_bash("/usr/bin/bash"));
    assert!(!runs_bash("/bin/sh"));
    assert!(!runs_bash("/usr/bin/zsh"));
}
