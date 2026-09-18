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

#[test]
fn check_lines_are_parsed_in_order_and_other_lines_ignored() {
    let checks = parse_checks(
        b"ok\truntime-cli\tpodman at /usr/bin/podman\t\nnoise without tabs\nwarn\tgh\tgh missing\tinstall gh\nfail\timage\timage x is not present\tbuild it\nbogus\tstatus\tx\ty\n",
    );
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
    assert!(
        parse_checks(b"ok\t\tdetail\t").is_empty(),
        "a nameless check is no check"
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
    let error = ScriptPreflight.preflight(&config(create)).unwrap_err();
    assert_eq!(
        error,
        "create script bash does not support --preflight-only or reported no checks (exit exit status: 1): legacy: unknown argument --preflight-only"
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
