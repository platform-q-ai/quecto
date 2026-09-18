//! Contract for the `ContainerRuntimePreflight` port (#2024 S4b): the
//! create script of the config is asked for its own checks with
//! `--preflight-only` appended and no child command; every status/check/
//! detail/remedy line it prints is a check, in order; a script that
//! refuses the flag, prints no checks, or is missing is an error carrying
//! its own words — never an empty, healthy report; nothing is created.
//! The port fails closed: a non-zero exit without a failed check (the
//! script died mid-report) is an error carrying its stderr, a malformed
//! check line refuses the report naming it, and a successful report must
//! carry the checks every shipped script makes (`MANDATORY_CHECKS`).
use std::path::Path;
use std::sync::Arc;

use quecto::application::environments::dto::{CheckStatus, DiagnosableContainerConfig};
use quecto::application::environments::ports::ContainerRuntimePreflight;
use quecto::composition::environments::build_container_runtime_preflight;
use quecto::infrastructure::processes::containers::preflight::MANDATORY_CHECKS;

fn port() -> Arc<dyn ContainerRuntimePreflight> {
    build_container_runtime_preflight()
}

/// A fixture script's argv, run through `bash` (a freshly written file
/// executed directly by a multi-threaded test binary can race a
/// concurrent fork holding it open, ETXTBSY).
fn script(dir: &Path, name: &str, body: &str) -> Vec<String> {
    let path = dir.join(name);
    std::fs::write(&path, format!("set -u\n{body}\n")).unwrap();
    vec!["bash".to_string(), path.to_string_lossy().into_owned()]
}

fn config(create: Vec<String>) -> DiagnosableContainerConfig {
    DiagnosableContainerConfig {
        name: "box".into(),
        create,
        diagnostics: vec![],
    }
}

#[test]
fn the_script_answers_with_its_checks_in_order_and_creates_nothing() {
    let dir = tempfile::TempDir::new().unwrap();
    let marker = dir.path().join("created");
    let create = script(
        dir.path(),
        "create.sh",
        &format!(
            r#"if [ "${{@: -1}}" != --preflight-only ]; then touch '{}'; exit 0; fi
[ -z "${{QUECTO_CONTAINER_ENVIRONMENT_REF:-}}" ] || exit 9
printf 'ok\truntime-cli\tpodman at /usr/bin/podman\t\n'
printf 'warn\tgh\tgh is not on PATH\tinstall gh\n'
printf 'fail\timage\timage quecto-box:local is not present\tbuild it (podman build -t quecto-box:local .)\n'
exit 1"#,
            marker.display()
        ),
    );
    let mut argv = create;
    argv.extend(["--state-dir".to_string(), "/s".to_string()]);
    let checks = port().preflight(&config(argv)).unwrap();
    let names: Vec<&str> = checks.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["runtime-cli", "gh", "image"]);
    assert_eq!(checks[0].status, CheckStatus::Passed);
    assert_eq!(checks[1].status, CheckStatus::Warned);
    assert_eq!(checks[1].remedy, "install gh");
    assert_eq!(checks[2].status, CheckStatus::Failed);
    assert_eq!(checks[2].detail, "image quecto-box:local is not present");
    assert!(!marker.exists(), "a preflight never creates an environment");
}

#[test]
fn a_script_that_refuses_the_flag_is_an_error_carrying_its_own_words() {
    let dir = tempfile::TempDir::new().unwrap();
    let create = script(
        dir.path(),
        "legacy.sh",
        "echo 'legacy create: unknown argument --preflight-only' >&2; exit 2",
    );
    let error = port().preflight(&config(create.clone())).unwrap_err();
    assert!(error.contains(&create.join(" ")), "{error}");
    assert!(
        error.contains("does not support --preflight-only"),
        "{error}"
    );
    assert!(
        error.contains("legacy create: unknown argument --preflight-only"),
        "{error}"
    );
}

#[test]
fn a_script_that_dies_mid_report_is_an_error_carrying_its_stderr() {
    let dir = tempfile::TempDir::new().unwrap();
    let dies = script(
        dir.path(),
        "dies.sh",
        "printf 'ok\\truntime-cli\\tpodman\\t\\nok\\tjq\\tjq\\t\\n'; echo 'usage: bad timeout' >&2; exit 2",
    );
    let error = port().preflight(&config(dies)).unwrap_err();
    assert!(
        error.contains("exited exit status: 2 after 2 checks without reporting a failure")
            && error.ends_with(": usage: bad timeout"),
        "{error}"
    );
}

#[test]
fn a_malformed_check_line_refuses_the_report_naming_it() {
    let dir = tempfile::TempDir::new().unwrap();
    let malformed = script(
        dir.path(),
        "malformed.sh",
        "printf 'ok\\tjq\\tjq\\t\\nfail\\timage\\n'; exit 1",
    );
    let error = port().preflight(&config(malformed)).unwrap_err();
    assert!(
        error.contains("malformed check line \"fail\\timage\""),
        "{error}"
    );
}

#[test]
fn a_successful_report_must_carry_every_mandatory_check() {
    assert_eq!(MANDATORY_CHECKS, ["jq", "git", "repo", "state-dir"]);
    let dir = tempfile::TempDir::new().unwrap();
    let lines: String = MANDATORY_CHECKS
        .iter()
        .map(|name| format!("ok\\t{name}\\tfine\\t\\n"))
        .collect();
    let complete = script(
        dir.path(),
        "complete.sh",
        &format!("printf '{lines}'; exit 0"),
    );
    assert_eq!(port().preflight(&config(complete)).unwrap().len(), 4);
    let short = script(
        dir.path(),
        "short.sh",
        "printf 'ok\\tjq\\tjq\\t\\nok\\tgit\\tgit\\t\\nok\\trepo\\tfine\\t\\n'; exit 0",
    );
    let error = port().preflight(&config(short)).unwrap_err();
    assert!(
        error.contains("without reporting the mandatory check state-dir"),
        "{error}"
    );
}

#[test]
fn a_silent_script_and_a_missing_one_are_errors_not_healthy_reports() {
    let dir = tempfile::TempDir::new().unwrap();
    let silent = script(dir.path(), "silent.sh", "exit 0");
    let error = port().preflight(&config(silent)).unwrap_err();
    assert!(error.contains("reported no checks"), "{error}");
    let error = port()
        .preflight(&config(vec!["/definitely/not/create".into()]))
        .unwrap_err();
    assert!(error.contains("failed to invoke"), "{error}");
    let error = port().preflight(&config(vec![])).unwrap_err();
    assert!(!error.is_empty());
}
