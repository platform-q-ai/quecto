use super::*;

/// The create argv of a script run through `bash` (a file executed
/// directly by a multi-threaded test process can race a concurrent fork
/// holding it open, ETXTBSY).
fn script(dir: &std::path::Path, name: &str, body: &str) -> Vec<String> {
    let path = dir.join(name);
    std::fs::write(&path, format!("{body}\n")).unwrap();
    vec!["bash".to_string(), path.to_string_lossy().into_owned()]
}

fn config(create: Vec<String>) -> DiagnosableContainerConfig {
    DiagnosableContainerConfig {
        name: "official".into(),
        create,
        diagnostics: vec![],
    }
}

/// The `ok` lines of every mandatory check, as a shell `printf` body.
const MANDATORY_OK_LINES: &str = "ok\\tjq\\tjq at /usr/bin/jq\\t\\nok\\tgit\\tgit at /usr/bin/git\\t\\nok\\trepo\\tnot needed\\t\\nok\\tstate-dir\\twritable\\t\\n";

#[test]
fn check_lines_are_parsed_in_order_and_tabless_lines_ignored() {
    let checks = parse_checks(
        b"ok\truntime-cli\tpodman at /usr/bin/podman\t\nnoise without tabs\nwarn\tgh\tgh missing\tinstall gh\nfail\timage\timage x is not present\tbuild it\n",
    )
    .unwrap();
    assert_eq!(
        checks,
        vec![
            PreflightCheck {
                name: "runtime-cli".into(),
                status: CheckStatus::Passed,
                detail: "podman at /usr/bin/podman".into(),
                remedy: String::new(),
            },
            PreflightCheck {
                name: "gh".into(),
                status: CheckStatus::Warned,
                detail: "gh missing".into(),
                remedy: "install gh".into(),
            },
            PreflightCheck {
                name: "image".into(),
                status: CheckStatus::Failed,
                detail: "image x is not present".into(),
                remedy: "build it".into(),
            },
        ]
    );
}

#[test]
fn a_check_shaped_line_that_is_not_a_check_refuses_the_report_naming_it() {
    for (stdout, malformed) in [
        (&b"ok\tjq\tfine\t\nfail\timage\n"[..], "fail\timage"),
        (b"fail\timage", "fail\timage"),
        (b"ok\t\tdetail\t", "ok\t\tdetail\t"),
        (b"bogus\tstatus\tx\ty", "bogus\tstatus\tx\ty"),
        (b"OK\tjq\tfine\t", "OK\tjq\tfine\t"),
    ] {
        assert_eq!(parse_checks(stdout).unwrap_err(), malformed);
    }
    assert_eq!(
        parse_checks(b"ok\tjq\tfine").unwrap().len(),
        1,
        "three fields (no remedy) are a check"
    );
}

#[test]
fn a_script_that_dies_after_some_ok_lines_is_an_error_carrying_its_stderr() {
    let dir = tempfile::TempDir::new().unwrap();
    let create = script(
        dir.path(),
        "dies.sh",
        "printf 'ok\\truntime-cli\\tpodman\\t\\nok\\tjq\\tjq\\t\\n'\necho 'create: QUECTO_REPO_CHECK_TIMEOUT must be a positive integer (seconds)' >&2\nexit 2",
    );
    let error = ScriptPreflight
        .preflight(&config(create.clone()))
        .unwrap_err();
    assert_eq!(
        error,
        format!(
            "create script `{}` exited exit status: 2 after 2 checks without reporting a failure: create: QUECTO_REPO_CHECK_TIMEOUT must be a positive integer (seconds)",
            create.join(" ")
        )
    );
}

#[test]
fn a_successful_report_missing_a_mandatory_check_is_an_error() {
    let dir = tempfile::TempDir::new().unwrap();
    let create = script(
        dir.path(),
        "short.sh",
        "printf 'ok\\tjq\\tjq\\t\\nok\\tgit\\tgit\\t\\n'\nexit 0",
    );
    let error = ScriptPreflight
        .preflight(&config(create.clone()))
        .unwrap_err();
    assert_eq!(
        error,
        format!(
            "create script `{}` exited 0 without reporting the mandatory checks repo, state-dir (a report cut short is not a healthy one)",
            create.join(" ")
        )
    );
    let complete = script(
        dir.path(),
        "complete.sh",
        &format!("printf '{MANDATORY_OK_LINES}'\nexit 0"),
    );
    assert_eq!(
        ScriptPreflight.preflight(&config(complete)).unwrap().len(),
        4
    );
}

#[test]
fn a_malformed_line_refuses_the_whole_report() {
    let dir = tempfile::TempDir::new().unwrap();
    let create = script(
        dir.path(),
        "malformed.sh",
        &format!("printf '{MANDATORY_OK_LINES}fail\\timage\\n'\nexit 1"),
    );
    let error = ScriptPreflight
        .preflight(&config(create.clone()))
        .unwrap_err();
    assert_eq!(
        error,
        format!(
            "create script `{}` printed a malformed check line \"fail\\timage\" (expected `ok|warn|fail<TAB>check<TAB>detail[<TAB>remedy]`)",
            create.join(" ")
        )
    );
}

#[test]
fn the_argv_and_the_stderr_of_a_report_never_carry_a_repo_token() {
    let dir = tempfile::TempDir::new().unwrap();
    let mut create = script(
        dir.path(),
        "leaky.sh",
        "echo \"create: cloning $2 failed\" >&2\nexit 2",
    );
    create.extend(["--repo".into(), "https://user:ghp_secret@host/x/y".into()]);
    let error = ScriptPreflight.preflight(&config(create)).unwrap_err();
    assert!(!error.contains("ghp_secret"), "{error}");
    assert!(
        error.contains("--repo https://***@host/x/y")
            && error.contains("cloning https://***@host/x/y failed"),
        "{error}"
    );
}

#[test]
fn the_script_is_run_with_the_flag_appended_and_its_lines_are_the_checks() {
    let dir = tempfile::TempDir::new().unwrap();
    let recorder = dir.path().join("argv");
    let create = script(
        dir.path(),
        "create.sh",
        &format!(
            "printf '%s\\n' \"$@\" > '{}'\nprintf 'env=%s\\n' \"${{QUECTO_CONTAINER_CONFIG:-}}\" >> '{}'\nprintf 'ok\\trepo\\treachable\\t\\nfail\\timage\\timage q is not present\\tbuild it\\n'\nexit 1",
            recorder.display(),
            recorder.display()
        ),
    );
    let mut argv = create;
    argv.extend(["--state-dir".to_string(), "/s".to_string()]);
    let checks = ScriptPreflight.preflight(&config(argv)).unwrap();
    assert_eq!(checks.len(), 2);
    assert_eq!(checks[1].status, CheckStatus::Failed);
    assert_eq!(
        std::fs::read_to_string(&recorder).unwrap(),
        "--state-dir\n/s\n--preflight-only\nenv=official\n"
    );
}

#[test]
fn a_script_that_refuses_the_flag_reports_its_own_words() {
    let dir = tempfile::TempDir::new().unwrap();
    let create = script(
        dir.path(),
        "legacy.sh",
        "echo 'legacy: unknown argument --preflight-only' >&2\nexit 1",
    );
    let error = ScriptPreflight
        .preflight(&config(create.clone()))
        .unwrap_err();
    assert_eq!(
        error,
        format!(
            "create script `{}` does not support --preflight-only or reported no checks (exit exit status: 1): legacy: unknown argument --preflight-only",
            create.join(" ")
        )
    );
}

#[test]
fn a_missing_script_and_an_empty_argv_are_errors() {
    let error = ScriptPreflight
        .preflight(&config(vec!["/definitely/not/create".into()]))
        .unwrap_err();
    assert!(error.contains("failed to invoke script"), "{error}");
    assert_eq!(
        ScriptPreflight.preflight(&config(vec![])).unwrap_err(),
        "the config has no create argv"
    );
}
