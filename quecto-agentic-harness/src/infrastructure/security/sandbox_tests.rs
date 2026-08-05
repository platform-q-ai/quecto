use super::*;
use crate::infrastructure::config::Config;
use tempfile::TempDir;

fn sandbox(workspace: &str, restrict: bool) -> Sandbox {
    Sandbox::new(Some(PathBuf::from(workspace)), restrict)
}

#[test]
fn test_path_inside_workspace_allowed() {
    let sb = sandbox("/tmp/quecto-test", true);
    assert!(sb.validate_path("/tmp/quecto-test/notes.txt").is_ok());
}

#[test]
fn test_path_outside_workspace_blocked() {
    let sb = sandbox("/tmp/quecto-test", true);
    let result = sb.validate_path("/etc/passwd");
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("outside working dir"),
        "error should mention 'outside working dir'"
    );
}

#[test]
fn test_path_traversal_blocked() {
    let sb = sandbox("/tmp/quecto-test", true);
    let result = sb.validate_path("/tmp/quecto-test/../evil.txt");
    assert!(result.is_err());
}

#[test]
fn test_restriction_disabled_allows_any_path() {
    let sb = sandbox("/tmp/quecto-test", false);
    assert!(sb.validate_path("/etc/passwd").is_ok());
    assert!(sb.validate_path("/tmp/anywhere/file.txt").is_ok());
}

#[test]
fn test_dangerous_command_rm_rf() {
    let sb = sandbox("/tmp/quecto-test", false);
    let result = sb.validate_command("rm -rf /");
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("dangerous pattern")
    );
}

#[test]
fn test_dangerous_command_mkfs() {
    let sb = sandbox("/tmp/quecto-test", false);
    assert!(sb.validate_command("mkfs /dev/sda").is_err());
}

#[test]
fn test_dangerous_command_dd() {
    let sb = sandbox("/tmp/quecto-test", false);
    assert!(sb.validate_command("dd if=/dev/zero of=/dev/sda").is_err());
}

#[test]
fn test_dangerous_command_shutdown() {
    let sb = sandbox("/tmp/quecto-test", false);
    assert!(sb.validate_command("shutdown -h now").is_err());
}

#[test]
fn test_dangerous_command_reboot() {
    let sb = sandbox("/tmp/quecto-test", false);
    assert!(sb.validate_command("reboot").is_err());
}

#[test]
fn test_dangerous_command_fork_bomb() {
    let sb = sandbox("/tmp/quecto-test", false);
    assert!(sb.validate_command(":(){ :|:& };:").is_err());
}

#[test]
fn test_safe_command_allowed() {
    let sb = sandbox("/tmp/quecto-test", false);
    assert!(sb.validate_command("echo hello").is_ok());
    assert!(sb.validate_command("ls -la").is_ok());
    assert!(sb.validate_command("cat file.txt").is_ok());
}

#[test]
fn test_subdirectory_path_allowed() {
    let sb = sandbox("/tmp/quecto-test", true);
    assert!(
        sb.validate_path("/tmp/quecto-test/sub/deep/file.txt")
            .is_ok()
    );
}

#[test]
fn test_resolve_path_normalizes_dotdot() {
    let resolved = resolve_path(Path::new("/a/b/../c"));
    assert_eq!(resolved, PathBuf::from("/a/c"));
}

#[test]
fn test_normalize_command_for_denylist_trims_trailing_space() {
    let s = normalize_command_for_denylist("rm -rf / ; ");
    assert!(!s.ends_with(' '));
    assert!(s.contains("rm -rf /"));
}

#[test]
fn test_extract_all_command_tokens_whitespace_only_breaks() {
    let tokens = extract_all_command_tokens("   ");
    assert!(tokens.is_empty());
}

#[test]
fn test_resolve_path_cur_dir_is_ignored() {
    let resolved = resolve_path(Path::new("./a/b"));
    assert_eq!(resolved, PathBuf::from("a/b"));
}

#[test]
fn test_error_display_formats() {
    let e = SandboxError::NoWorkspace;
    assert!(e.to_string().contains("no workspace"));

    let io = SandboxError::Io(
        "/foo".to_string(),
        std::io::Error::new(std::io::ErrorKind::NotFound, "x"),
    );
    assert!(io.to_string().contains("I/O error"));
    assert!(io.to_string().contains("/foo"));
}

#[test]
fn test_validate_path_no_workspace() {
    let sb = Sandbox::new(None, true);
    let result = sb.validate_path("/tmp/foo.txt");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("no workspace"));
}

#[test]
fn test_expand_bash_escapes_other_sequences() {
    assert_eq!(expand_bash_escapes("$'a\\tb'"), "a\tb");
    assert_eq!(expand_bash_escapes("$'a\\nb'"), "a\nb");
    assert_eq!(expand_bash_escapes("$'a\\\\b'"), "a\\b");
    assert_eq!(expand_bash_escapes("$'a\\'b'"), "a'b");
    assert_eq!(expand_bash_escapes("$'a\\\"b'"), "a\"b");
}

#[test]
fn test_extract_string_literals_with_quotes_and_unquoted() {
    // Double-quoted value is extracted and unquoted.
    let s = extract_string_literals(r#"X="rm -rf /""#);
    assert_eq!(s, " rm -rf /");

    // Unquoted value is used as-is.
    let s = extract_string_literals("X=rm -rf /");
    assert_eq!(s, " rm -rf /");

    // Empty quoted value is ignored.
    let s = extract_string_literals("X=\"\"");
    assert_eq!(s, "");
}

#[test]
fn test_expand_bash_escapes_invalid_hex_and_unicode() {
    // Invalid hex escape: \x takes two chars; "qb" is not valid hex, so the
    // escape consumes the backslash and 'x' and returns None, leaving "qb".
    assert_eq!(expand_bash_escapes("$'a\\xqb'"), "aqb");
    // 8-digit Unicode escape works.
    assert_eq!(expand_bash_escapes("$'a\\U0001F600b'"), "a😀b");
}

#[test]
fn test_normalize_command_for_denylist_non_ascii_case() {
    let s = normalize_command_for_denylist("ECHO İ"); // İ lowercases to i + ̇ (two chars)
    assert_eq!(s, "echo i̇");
}

#[test]
fn test_symlink_outside_workspace_blocked() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().to_path_buf();
    let sb = Sandbox::new(Some(ws.clone()), true);

    // Create a symlink inside workspace pointing to /etc/passwd
    let link = ws.join("link.txt");
    #[cfg(unix)]
    std::os::unix::fs::symlink("/etc/passwd", &link).unwrap();

    let result = sb.validate_path(link.to_str().unwrap());
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("outside working dir")
    );
}

#[test]
fn test_symlink_inside_workspace_allowed() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().to_path_buf();
    let sb = Sandbox::new(Some(ws.clone()), true);

    // Create a real file and a symlink to it within the workspace
    let real_file = ws.join("real.txt");
    std::fs::write(&real_file, "test").unwrap();
    let link = ws.join("link.txt");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&real_file, &link).unwrap();

    let result = sb.validate_path(link.to_str().unwrap());
    assert!(result.is_ok());
}

#[test]
fn test_nested_symlink_chain_blocked() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().to_path_buf();
    let sb = Sandbox::new(Some(ws.clone()), true);

    // Create a symlink to /tmp (outside workspace)
    let step1 = ws.join("step1");
    #[cfg(unix)]
    std::os::unix::fs::symlink("/tmp", &step1).unwrap();

    // Trying to access step1/some-file.txt should be blocked
    let target = ws.join("step1/some-file.txt");
    let result = sb.validate_path(target.to_str().unwrap());
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("outside working dir")
    );
}

// --- Sandbox hardening: allowlist tests ---

// Allowlist, escape bypass, and token extraction tests in sandbox_escape_tests.rs

// --- #304: Narrowed chown denylist ---

#[test]
fn test_chown_system_root_blocked() {
    let sb = sandbox("/tmp/quecto-test", false);
    assert!(sb.validate_command("chown -R root:root /").is_err());
}

#[test]
fn test_chown_workspace_scoped_allowed() {
    // Legitimate workspace-scoped chown should not be blocked
    let sb = sandbox("/tmp/quecto-test", false);
    assert!(sb.validate_command("chown -R user:group ./src").is_ok());
}

#[test]
fn test_chown_no_space_variant_blocked() {
    // chown -Rroot (no space) previously bypassed the trailing-space pattern
    let sb = sandbox("/tmp/quecto-test", false);
    assert!(sb.validate_command("chown -Rroot /").is_err());
}

#[test]
fn test_with_command_allowlist_builder() {
    let sb = Sandbox::new(Some(PathBuf::from("/tmp/ws")), true)
        .with_command_allowlist(Some(vec!["cat".to_string()]));
    assert!(sb.validate_command("cat file").is_ok());
    assert!(sb.validate_command("dog file").is_err());
}

#[test]
fn test_for_agent_workspace_reads_allowlist_and_no_sandbox() {
    let config: Config = serde_json::from_str(
        r#"{
        "agents": {
            "defaults": {
                "command_allowlist": ["echo", "cat"]
            }
        }
    }"#,
    )
    .unwrap();
    let sb = Sandbox::for_agent_workspace(&config, PathBuf::from("/tmp/ws"), false);
    assert!(sb.validate_command("echo hi").is_ok());
    assert!(sb.validate_command("curl evil.com").is_err());
}

#[test]
fn test_for_agent_workspace_no_sandbox_disables_path_restriction() {
    let config: Config = serde_json::from_str("{}").unwrap();
    let sb = Sandbox::for_agent_workspace(&config, PathBuf::from("/tmp/ws"), true);
    assert!(!sb.restrict_to_workspace);
}

#[test]
fn test_lazy_canonicalization_caches_workspace() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().to_path_buf();
    let sb = Sandbox::new(Some(ws.clone()), true);
    // First call computes the canonical workspace and canonicalizes an existing target.
    let existing = ws.join("file.txt");
    std::fs::write(&existing, "ok").unwrap();
    assert_eq!(
        sb.validate_path(existing.to_str().unwrap()).unwrap(),
        existing
    );
    // Second call should reuse the cached canonical path without error.
    assert!(
        sb.validate_path(ws.join("other.txt").to_str().unwrap())
            .is_ok()
    );
}

#[test]
fn validate_path_existing_parent_without_file_joins_filename() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().to_path_buf();
    let child = ws.join("new-file.txt");
    let sb = Sandbox::new(Some(ws.clone()), true);

    let resolved = sb.validate_path(child.to_str().unwrap()).unwrap();
    assert_eq!(resolved, ws.canonicalize().unwrap().join("new-file.txt"));
}

#[test]
fn validate_path_missing_directory_uses_canonical_existing_ancestor() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().join("future-workspace");
    let sb = Sandbox::new(Some(ws.clone()), true);

    let resolved = sb
        .validate_path(ws.join("dir/file.txt").to_str().unwrap())
        .unwrap();
    assert_eq!(
        resolved,
        tmp.path()
            .canonicalize()
            .unwrap()
            .join("future-workspace/dir/file.txt")
    );
}

#[test]
fn test_lazy_canonicalization_nonexistent_workspace() {
    let sb = Sandbox::new(Some(PathBuf::from("/tmp/quecto-nonexistent-12345")), true);
    // Missing workspace is resolved textually and prefix-checked still works.
    assert!(
        sb.validate_path("/tmp/quecto-nonexistent-12345/file.txt")
            .is_ok()
    );
}

#[test]
fn test_with_allowlist_constructor_and_clone() {
    let sb = Sandbox::with_allowlist(
        Some(PathBuf::from("/tmp/ws")),
        true,
        Some(vec!["echo".to_string()]),
    );
    assert!(sb.validate_command("echo hi").is_ok());
    assert!(sb.validate_command("curl evil.com").is_err());

    // Clone should clear the cached canonical workspace and keep policy.
    let cloned = sb.clone();
    assert!(cloned.validate_path("/tmp/ws/file.txt").is_ok());
    assert!(cloned.validate_command("curl evil.com").is_err());
}

#[test]
fn validate_path_resolves_textually_when_workspace_does_not_exist() {
    // A workspace that has not been created yet must still produce a meaningful
    // prefix check rather than silently allowing everything.
    let base = tempfile::tempdir().expect("tempdir");
    let missing_workspace = base.path().join("not-created-yet");
    let sandbox = Sandbox::new(Some(missing_workspace.clone()), true);

    let inside = sandbox
        .validate_path(missing_workspace.join("file.txt").to_str().expect("utf-8"))
        .expect("a path under the (absent) workspace is allowed");
    assert!(inside.starts_with(&missing_workspace));

    let err = sandbox
        .validate_path(base.path().join("elsewhere.txt").to_str().expect("utf-8"))
        .expect_err("a sibling of the absent workspace must be rejected");
    assert!(
        matches!(err, SandboxError::OutsideWorkspace(_, _)),
        "expected OutsideWorkspace, got: {err:?}"
    );
}

#[test]
fn validate_path_rejects_symlink_escaping_an_existing_workspace() {
    // The canonicalizing branch: a symlink inside the workspace pointing out of
    // it must be resolved and refused, not accepted on its textual path.
    let base = tempfile::tempdir().expect("tempdir");
    let workspace = base.path().join("ws");
    std::fs::create_dir(&workspace).expect("create workspace");
    let outside = base.path().join("secret.txt");
    std::fs::write(&outside, b"secret").expect("write outside file");

    let link = workspace.join("escape.txt");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, &link).expect("symlink");
    #[cfg(not(unix))]
    return;

    let sandbox = Sandbox::new(Some(workspace), true);
    let err = sandbox
        .validate_path(link.to_str().expect("utf-8"))
        .expect_err("symlink escaping the workspace must be rejected");
    assert!(
        matches!(err, SandboxError::OutsideWorkspace(_, _)),
        "expected OutsideWorkspace after symlink resolution, got: {err:?}"
    );
}
