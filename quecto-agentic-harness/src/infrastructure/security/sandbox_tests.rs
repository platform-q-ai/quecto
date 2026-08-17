use super::*;
use crate::infrastructure::config::Config;
use tempfile::TempDir;

fn sandbox(workspace: &str) -> Sandbox {
    Sandbox::new(Some(PathBuf::from(workspace)))
}

#[test]
fn validate_path_allows_absolute_outside_workspace() {
    let sb = sandbox("/tmp/quecto-test");
    assert_eq!(
        sb.validate_path("/etc/passwd").unwrap(),
        PathBuf::from("/etc/passwd")
    );
}

#[test]
fn validate_path_allows_parent_traversal_textually() {
    let sb = sandbox("/tmp/quecto-test");
    assert_eq!(
        sb.validate_path("/tmp/quecto-test/../evil.txt").unwrap(),
        PathBuf::from("/tmp/quecto-test/../evil.txt")
    );
}

#[test]
fn validate_path_allows_symlink_outside_workspace() {
    let tmp = TempDir::new().unwrap();
    let ws = tmp.path().to_path_buf();
    let sb = Sandbox::new(Some(ws.clone()));
    let link = ws.join("link.txt");
    #[cfg(unix)]
    std::os::unix::fs::symlink("/etc/passwd", &link).unwrap();
    assert!(sb.validate_path(link.to_str().unwrap()).is_ok());
}

#[test]
fn validate_path_no_workspace_still_allows_path() {
    let sb = Sandbox::new(None);
    assert_eq!(
        sb.validate_path("/tmp/foo.txt").unwrap(),
        PathBuf::from("/tmp/foo.txt")
    );
}

#[test]
fn test_dangerous_command_rm_rf() {
    let sb = sandbox("/tmp/quecto-test");
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
    assert!(
        sandbox("/tmp/quecto-test")
            .validate_command("mkfs /dev/sda")
            .is_err()
    );
}

#[test]
fn test_dangerous_command_dd() {
    assert!(
        sandbox("/tmp/quecto-test")
            .validate_command("dd if=/dev/zero of=/dev/sda")
            .is_err()
    );
}

#[test]
fn test_dangerous_command_shutdown() {
    assert!(
        sandbox("/tmp/quecto-test")
            .validate_command("shutdown -h now")
            .is_err()
    );
}

#[test]
fn test_dangerous_command_reboot() {
    assert!(
        sandbox("/tmp/quecto-test")
            .validate_command("reboot")
            .is_err()
    );
}

#[test]
fn test_dangerous_command_fork_bomb() {
    assert!(
        sandbox("/tmp/quecto-test")
            .validate_command(":(){ :|:& };:")
            .is_err()
    );
}

#[test]
fn test_safe_command_allowed() {
    let sb = sandbox("/tmp/quecto-test");
    assert!(sb.validate_command("echo hello").is_ok());
    assert!(sb.validate_command("ls -la").is_ok());
    assert!(sb.validate_command("cat file.txt").is_ok());
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
fn test_error_display_formats() {
    let e = SandboxError::DangerousPattern("rm -rf /".into(), "rm -rf /".into());
    assert!(e.to_string().contains("dangerous pattern"));
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
    assert_eq!(extract_string_literals(r#"X="rm -rf /""#), " rm -rf /");
    assert_eq!(extract_string_literals("X=rm -rf /"), " rm -rf /");
    assert_eq!(extract_string_literals("X=\"\""), "");
}

#[test]
fn test_expand_bash_escapes_invalid_hex_and_unicode() {
    assert_eq!(expand_bash_escapes("$'a\\xqb'"), "aqb");
    assert_eq!(expand_bash_escapes("$'a\\U0001F600b'"), "a😀b");
}

#[test]
fn test_normalize_command_for_denylist_non_ascii_case() {
    assert_eq!(normalize_command_for_denylist("ECHO İ"), "echo i̇");
}

#[test]
fn test_chown_system_root_blocked() {
    assert!(
        sandbox("/tmp/quecto-test")
            .validate_command("chown -R root:root /")
            .is_err()
    );
}

#[test]
fn test_chown_workspace_scoped_allowed() {
    assert!(
        sandbox("/tmp/quecto-test")
            .validate_command("chown -R user:group ./src")
            .is_ok()
    );
}

#[test]
fn test_chown_no_space_variant_blocked() {
    assert!(
        sandbox("/tmp/quecto-test")
            .validate_command("chown -Rroot /")
            .is_err()
    );
}

#[test]
fn test_with_command_allowlist_builder() {
    let sb = Sandbox::new(Some(PathBuf::from("/tmp/ws")))
        .with_command_allowlist(Some(vec!["cat".to_string()]));
    assert!(sb.validate_command("cat file").is_ok());
    assert!(sb.validate_command("dog file").is_err());
}

#[test]
fn test_for_agent_workspace_reads_allowlist() {
    let config: Config =
        serde_json::from_str(r#"{"agents":{"defaults":{"command_allowlist":["echo","cat"]}}}"#)
            .unwrap();
    let sb = Sandbox::for_agent_workspace(&config, PathBuf::from("/tmp/ws"));
    assert!(sb.validate_command("echo hi").is_ok());
    assert!(sb.validate_command("curl evil.com").is_err());
}

#[test]
fn test_with_allowlist_constructor_and_clone() {
    let sb = Sandbox::with_allowlist(
        Some(PathBuf::from("/tmp/ws")),
        Some(vec!["echo".to_string()]),
    );
    assert!(sb.validate_command("echo hi").is_ok());
    assert!(sb.validate_command("curl evil.com").is_err());
    let cloned = sb.clone();
    assert!(cloned.validate_path("/tmp/elsewhere/file.txt").is_ok());
    assert!(cloned.validate_command("curl evil.com").is_err());
}
