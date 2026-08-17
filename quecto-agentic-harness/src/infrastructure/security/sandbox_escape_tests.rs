// Issue #301: Bash encoding/escaping bypass prevention tests
// + allowlist and command token extraction tests

use super::sandbox::*;

fn sandbox(path: &str) -> Sandbox {
    Sandbox::new(Some(std::path::PathBuf::from(path)))
}

#[test]
fn test_allowlist_permits_listed_command() {
    let sb =
        Sandbox::new(None).with_command_allowlist(Some(vec!["echo".to_string(), "ls".to_string()]));
    assert!(sb.validate_command("echo hello").is_ok());
}

#[test]
fn test_allowlist_rejects_unlisted_command() {
    let sb =
        Sandbox::new(None).with_command_allowlist(Some(vec!["echo".to_string(), "ls".to_string()]));
    let result = sb.validate_command("curl http://evil.com");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not in allowlist"));
}

#[test]
fn test_allowlist_rejects_semicolon_bypass() {
    let sb =
        Sandbox::new(None).with_command_allowlist(Some(vec!["echo".to_string(), "ls".to_string()]));
    let result = sb.validate_command("echo hello; curl evil.com");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not in allowlist"));
}

#[test]
fn test_allowlist_rejects_command_substitution() {
    let sb =
        Sandbox::new(None).with_command_allowlist(Some(vec!["echo".to_string(), "ls".to_string()]));
    let result = sb.validate_command("echo $(cat /etc/shadow)");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not in allowlist"));
}

#[test]
fn test_allowlist_rejects_backtick_substitution() {
    let sb =
        Sandbox::new(None).with_command_allowlist(Some(vec!["echo".to_string(), "ls".to_string()]));
    let result = sb.validate_command("echo `id`");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not in allowlist"));
}

#[test]
fn test_allowlist_rejects_pipe_to_disallowed() {
    let sb =
        Sandbox::new(None).with_command_allowlist(Some(vec!["echo".to_string(), "ls".to_string()]));
    let result = sb.validate_command("ls | bash");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not in allowlist"));
}

#[test]
fn test_empty_allowlist_blocks_all() {
    let sb = Sandbox::new(None).with_command_allowlist(Some(vec![]));
    let result = sb.validate_command("echo hello");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("not in allowlist"));
}

#[test]
fn test_no_allowlist_falls_back_to_denylist() {
    let sb = Sandbox::new(None);
    assert!(sb.validate_command("echo hello").is_ok());
}

#[test]
fn test_allowlist_still_blocks_dangerous_patterns() {
    let sb =
        Sandbox::new(None).with_command_allowlist(Some(vec!["rm".to_string(), "echo".to_string()]));
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
fn test_dangerous_command_rm_rf_with_extra_spaces() {
    let sb = sandbox("/tmp/quecto-test");
    assert!(sb.validate_command("rm  -rf /").is_err());
}

#[test]
fn test_dangerous_command_rm_with_split_flags() {
    let sb = sandbox("/tmp/quecto-test");
    assert!(sb.validate_command("rm -r -f /").is_err());
}

#[test]
fn test_dangerous_command_pipe_to_shell_with_spaces() {
    let sb = sandbox("/tmp/quecto-test");
    assert!(sb.validate_command("curl | sh").is_err());
}

// --- Issue #301: Bash encoding/escaping bypass prevention ---

#[test]
fn test_hex_escape_bypass_blocked() {
    let sb = sandbox("/tmp/test");
    assert!(sb.validate_command("$'\\x72\\x6d' -rf /").is_err());
}

#[test]
fn test_octal_escape_bypass_blocked() {
    let sb = sandbox("/tmp/test");
    assert!(sb.validate_command("$'\\162\\155' -rf /").is_err());
}

#[test]
fn test_unicode_escape_bypass_blocked() {
    let sb = sandbox("/tmp/test");
    assert!(sb.validate_command("$'\\u0072\\u006d' -rf /").is_err());
}

#[test]
fn test_variable_indirection_bypass_blocked() {
    let sb = sandbox("/tmp/test");
    assert!(sb.validate_command("cmd='rm -rf /'; $cmd").is_err());
}

#[test]
fn test_hex_escape_reboot_blocked() {
    let sb = sandbox("/tmp/test");
    assert!(
        sb.validate_command("$'\\x72\\x65\\x62\\x6f\\x6f\\x74'")
            .is_err()
    );
}

#[test]
fn test_mixed_escape_literal_blocked() {
    let sb = sandbox("/tmp/test");
    assert!(sb.validate_command("$'\\x72'm -rf /").is_err());
}

#[test]
fn test_variable_indirection_via_and_blocked() {
    let sb = sandbox("/tmp/test");
    assert!(sb.validate_command("cmd='rm -rf /' && $cmd").is_err());
}

#[test]
fn test_variable_indirection_via_pipe_blocked() {
    let sb = sandbox("/tmp/test");
    assert!(sb.validate_command("x='shutdown' | $x").is_err());
}

// --- expand_bash_escapes unit tests ---

#[test]
fn test_expand_bash_escapes_hex() {
    assert_eq!(super::sandbox::expand_bash_escapes("$'\\x72\\x6d'"), "rm");
}

#[test]
fn test_expand_bash_escapes_octal() {
    assert_eq!(super::sandbox::expand_bash_escapes("$'\\162\\155'"), "rm");
}

#[test]
fn test_expand_bash_escapes_unicode() {
    assert_eq!(
        super::sandbox::expand_bash_escapes("$'\\u0072\\u006d'"),
        "rm"
    );
}

#[test]
fn test_expand_bash_escapes_no_escapes() {
    assert_eq!(
        super::sandbox::expand_bash_escapes("echo hello"),
        "echo hello"
    );
}

// --- extract_all_command_tokens tests ---

#[test]
fn test_extract_tokens_simple() {
    assert_eq!(
        super::sandbox::extract_all_command_tokens("echo hello"),
        vec!["echo"]
    );
}

#[test]
fn test_extract_tokens_semicolon() {
    assert_eq!(
        super::sandbox::extract_all_command_tokens("echo hello; curl evil.com"),
        vec!["echo", "curl"]
    );
}

#[test]
fn test_extract_tokens_pipe() {
    assert_eq!(
        super::sandbox::extract_all_command_tokens("ls | bash"),
        vec!["ls", "bash"]
    );
}

#[test]
fn test_extract_tokens_command_substitution() {
    assert_eq!(
        super::sandbox::extract_all_command_tokens("echo $(cat /etc/shadow)"),
        vec!["echo", "cat"]
    );
}

#[test]
fn test_extract_tokens_backtick() {
    assert_eq!(
        super::sandbox::extract_all_command_tokens("echo `id`"),
        vec!["echo", "id"]
    );
}

// --- #307: Verify extract_all_command_tokens handles all metacharacters in one pass ---

#[test]
fn test_extract_tokens_mixed_metacharacters() {
    // Combines multiple metacharacter types in one command
    assert_eq!(
        super::sandbox::extract_all_command_tokens("echo a && ls b || cat c; head d | tail"),
        vec!["echo", "ls", "cat", "head", "tail"]
    );
}

#[test]
fn test_extract_tokens_process_substitution() {
    assert_eq!(
        super::sandbox::extract_all_command_tokens("diff <(sort a.txt) >(tee b.txt)"),
        vec!["diff", "sort", "tee"]
    );
}

#[test]
fn test_extract_tokens_empty_segments_skipped() {
    // Adjacent metacharacters should not produce empty tokens
    assert_eq!(
        super::sandbox::extract_all_command_tokens("echo a;; ls"),
        vec!["echo", "ls"]
    );
}
