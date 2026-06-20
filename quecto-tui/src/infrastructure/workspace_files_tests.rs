use super::*;

#[test]
fn git_args_disable_config_defined_programs() {
    let args = git_ls_files_args(&[]);
    // Hardening overrides must precede the subcommand.
    let joined = args.join(" ");
    assert!(args.contains(&"core.fsmonitor=".to_string()), "{joined}");
    assert!(
        args.contains(&"core.hooksPath=/dev/null".to_string()),
        "{joined}"
    );
    let ls = args.iter().position(|a| a == "ls-files").unwrap();
    let fsmon = args.iter().position(|a| a == "core.fsmonitor=").unwrap();
    assert!(
        fsmon < ls,
        "-c overrides must come before ls-files: {joined}"
    );
    assert!(args.contains(&"-z".to_string()));
}

#[test]
fn git_args_append_extra_flags() {
    let args = git_ls_files_args(&["--others", "--exclude-standard"]);
    assert!(args.contains(&"--others".to_string()));
    assert!(args.contains(&"--exclude-standard".to_string()));
}

#[test]
fn is_safe_path_rejects_control_chars() {
    assert!(is_safe_path("src/main.rs"));
    assert!(is_safe_path("docs/a b.md")); // spaces are fine
    assert!(!is_safe_path("evil\x1b[2Jname.rs"), "ESC must be rejected");
    assert!(!is_safe_path("two\nlines.rs"), "newline must be rejected");
    assert!(!is_safe_path("tab\tname"), "tab must be rejected");
    assert!(!is_safe_path("\x7fdel"), "DEL must be rejected");
    assert!(!is_safe_path(""), "empty must be rejected");
}

#[test]
fn parse_git_output_splits_dedups_and_sanitizes() {
    let tracked = b"src/main.rs\0src/lib.rs\0bad\x1besc.rs\0".as_slice();
    let others = b"src/lib.rs\0README.md\0".as_slice();
    let files = parse_git_output(tracked, others);
    assert!(files.contains(&"src/main.rs".to_string()));
    assert!(files.contains(&"README.md".to_string()));
    assert_eq!(
        files.iter().filter(|f| f.as_str() == "src/lib.rs").count(),
        1,
        "duplicates across sources should be deduped"
    );
    assert!(
        !files.iter().any(|f| f.contains('\x1b')),
        "escape-bearing path must be dropped: {files:?}"
    );
}

#[test]
fn parse_git_output_caps_at_max() {
    // Build well over the cap; parsing must stop at MAX_WORKSPACE_FILES.
    let mut buf = Vec::new();
    for i in 0..(MAX_WORKSPACE_FILES + 500) {
        buf.extend_from_slice(format!("f{i:05}.rs\0").as_bytes());
    }
    let files = parse_git_output(&buf, &[]);
    assert_eq!(files.len(), MAX_WORKSPACE_FILES);
}

use std::path::PathBuf;

fn cov_tmp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "quecto-tui-wsfiles-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[test]
fn fs_walk_lists_files_skips_dotfiles_and_skip_dirs() {
    let root = cov_tmp("walk");
    std::fs::write(root.join("a.txt"), b"a").unwrap();
    std::fs::create_dir_all(root.join("sub")).unwrap();
    std::fs::write(root.join("sub/b.txt"), b"b").unwrap();
    // Skipped directories and dotfiles must not appear.
    std::fs::create_dir_all(root.join("target")).unwrap();
    std::fs::write(root.join("target/ignored.txt"), b"x").unwrap();
    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::write(root.join(".git/config"), b"x").unwrap();
    std::fs::write(root.join(".hidden"), b"x").unwrap();

    let files = fs_walk(&root);
    assert!(files.contains(&"a.txt".to_string()));
    assert!(files.contains(&"sub/b.txt".to_string()));
    assert!(!files.iter().any(|f| f.contains("target")));
    assert!(!files.iter().any(|f| f.contains(".git")));
    assert!(!files.iter().any(|f| f.contains(".hidden")));
    // Output is sorted.
    let mut sorted = files.clone();
    sorted.sort();
    assert_eq!(files, sorted);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn list_workspace_files_falls_back_to_fs_walk_when_not_git() {
    let root = cov_tmp("nogit");
    std::fs::write(root.join("only.rs"), b"fn main() {}").unwrap();
    let files = list_workspace_files(&root);
    assert_eq!(files, vec!["only.rs".to_string()]);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn git_files_lists_tracked_and_untracked() {
    if !git_available() {
        return;
    }
    let root = cov_tmp("git");
    let init = Command::new("git")
        .args(["init", "-q"])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(init.status.success());
    std::fs::write(root.join("tracked.rs"), b"x").unwrap();
    Command::new("git")
        .args(["add", "tracked.rs"])
        .current_dir(&root)
        .output()
        .unwrap();
    // Untracked-but-not-ignored file should also appear via --others.
    std::fs::write(root.join("untracked.txt"), b"y").unwrap();

    let files = git_files(&root).expect("git_files should return Some in a repo");
    assert!(files.contains(&"tracked.rs".to_string()), "{files:?}");
    assert!(files.contains(&"untracked.txt".to_string()), "{files:?}");

    // list_workspace_files should prefer the git source here.
    let listed = list_workspace_files(&root);
    assert!(listed.contains(&"tracked.rs".to_string()));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn run_git_returns_none_on_failure() {
    if !git_available() {
        return;
    }
    let root = cov_tmp("badgit");
    // Not a git repo → ls-files fails → None.
    assert!(run_git(&root, &git_ls_files_args(&[])).is_none());
    let _ = std::fs::remove_dir_all(&root);
}
