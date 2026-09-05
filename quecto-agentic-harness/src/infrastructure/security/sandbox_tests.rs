use super::*;

fn sandbox(workspace: &str) -> Sandbox {
    Sandbox::new(Some(std::path::PathBuf::from(workspace)))
}

#[test]
fn validate_path_allows_absolute_outside_workspace() {
    let sb = sandbox("/tmp/quecto-test");
    assert_eq!(
        sb.validate_path("/etc/passwd").unwrap(),
        std::path::PathBuf::from("/etc/passwd")
    );
}

#[test]
fn validate_path_allows_parent_traversal_textually() {
    let sb = sandbox("/tmp/quecto-test");
    assert_eq!(
        sb.validate_path("/tmp/quecto-test/../evil.txt").unwrap(),
        std::path::PathBuf::from("/tmp/quecto-test/../evil.txt")
    );
}

#[test]
fn validate_path_allows_symlink_outside_workspace() {
    let tmp = tempfile::tempdir().unwrap();
    let link = tmp.path().join("link.txt");
    #[cfg(unix)]
    std::os::unix::fs::symlink("/etc/passwd", &link).unwrap();
    let sb = Sandbox::new(Some(tmp.path().to_path_buf()));
    assert_eq!(sb.validate_path(link.to_str().unwrap()).unwrap(), link);
}

#[test]
fn validate_path_no_workspace_still_allows_path() {
    let sb = Sandbox::new(None);
    assert!(sb.validate_path("/etc/hosts").is_ok());
}

#[test]
fn test_dangerous_command_rm_rf() {
    let sb = sandbox("/tmp/quecto-test");
    let result = sb.validate_command("rm -rf /");
    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(msg.contains("dangerous pattern"), "{msg}");
    assert!(msg.contains("rm-root"), "{msg}");
    assert!(msg.contains("`rm -rf /`"), "{msg}");
}

#[test]
fn test_dangerous_command_mkfs() {
    let sb = sandbox("/tmp/quecto-test");
    assert!(sb.validate_command("mkfs /dev/sda").is_err());
}

#[test]
fn test_dangerous_command_dd() {
    let sb = sandbox("/tmp/quecto-test");
    assert!(sb.validate_command("dd if=/dev/zero of=/dev/sda").is_err());
}

#[test]
fn test_dangerous_command_shutdown() {
    let sb = sandbox("/tmp/quecto-test");
    assert!(sb.validate_command("shutdown -h now").is_err());
}

#[test]
fn test_dangerous_command_reboot() {
    let sb = sandbox("/tmp/quecto-test");
    assert!(sb.validate_command("reboot").is_err());
}

#[test]
fn test_dangerous_command_fork_bomb() {
    let sb = sandbox("/tmp/quecto-test");
    assert!(sb.validate_command(":(){ :|:& };:").is_err());
}

#[test]
fn test_safe_command_allowed() {
    let sb = sandbox("/tmp/quecto-test");
    assert!(sb.validate_command("echo hello").is_ok());
    assert!(sb.validate_command("ls -la").is_ok());
    assert!(sb.validate_command("cat file.txt").is_ok());
}

#[test]
fn test_prose_mentioning_dangerous_words_is_allowed() {
    let sb = sandbox("/tmp/quecto-test");
    assert!(sb.validate_command(r#"echo "the box will reboot""#).is_ok());
    assert!(sb.validate_command("grep halt notes.md").is_ok());
}

#[test]
fn test_error_display_formats() {
    let e = SandboxError::DangerousPattern {
        command: "reboot".into(),
        rule: "power-state".into(),
        site: "reboot".into(),
    };
    assert_eq!(
        e.to_string(),
        "command 'reboot' matches dangerous pattern 'power-state' at `reboot`"
    );
    assert!(format!("{e:?}").contains("DangerousPattern"));
}

#[test]
fn test_fallback_scan_error_names_the_reason() {
    let sb = Sandbox::new(None);
    let msg = sb
        .validate_command("cmd='rm -rf /'; $cmd")
        .unwrap_err()
        .to_string();
    assert!(msg.contains("fallback scan"), "{msg}");
    assert!(msg.contains("dynamic command name"), "{msg}");
}

#[test]
fn test_chown_system_root_blocked() {
    let sb = sandbox("/tmp/quecto-test");
    assert!(sb.validate_command("chown -R root:root /").is_err());
}

#[test]
fn test_chown_workspace_scoped_allowed() {
    let sb = sandbox("/tmp/quecto-test");
    assert!(sb.validate_command("chown -R user:group ./src").is_ok());
}

#[test]
fn test_chown_no_space_variant_blocked() {
    let sb = sandbox("/tmp/quecto-test");
    assert!(sb.validate_command("chown -Rroot /").is_err());
}

#[test]
fn test_for_agent_workspace_ignores_deprecated_allowlist() {
    let mut config = crate::infrastructure::config::Config::default();
    config.agents.defaults._deprecated_command_allowlist = Some(vec!["echo".to_string()]);
    let sb = Sandbox::for_agent_workspace(&config, std::path::PathBuf::from("/tmp/ws"));
    assert_eq!(sb.workspace, Some(std::path::PathBuf::from("/tmp/ws")));
    assert!(sb.validate_command("curl https://example.com").is_ok());
    assert!(sb.validate_command("rm -rf /").is_err());
}

#[test]
fn test_for_agent_workspace_without_deprecated_key() {
    let config = crate::infrastructure::config::Config::default();
    let sb = Sandbox::for_agent_workspace(&config, std::path::PathBuf::from("/tmp/ws"));
    assert!(sb.validate_command("echo hi").is_ok());
}

#[test]
fn test_clone_preserves_workspace() {
    let sb = sandbox("/tmp/ws");
    let cloned = sb.clone();
    assert_eq!(cloned.workspace, sb.workspace);
    assert!(cloned.validate_command("reboot").is_err());
}
