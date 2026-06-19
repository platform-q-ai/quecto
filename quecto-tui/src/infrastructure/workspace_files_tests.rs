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
