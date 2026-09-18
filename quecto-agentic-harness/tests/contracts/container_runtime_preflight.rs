//! Contract for the `ContainerRuntimePreflight` port (#2024 S4b): the
//! create script of the config is asked for its own checks with
//! `--preflight-only` appended and no child command; every status/check/
//! detail/remedy line it prints is a check, in order, whatever its exit
//! status; a script that refuses the flag, prints no checks, or is
//! missing is an error carrying its own words — never an empty, healthy
//! report; nothing is created.
use std::path::Path;
use std::sync::Arc;

use quecto::application::environments::dto::{CheckStatus, DiagnosableContainerConfig};
use quecto::application::environments::ports::ContainerRuntimePreflight;
use quecto::composition::environments::build_container_runtime_preflight;

fn port() -> Arc<dyn ContainerRuntimePreflight> {
    build_container_runtime_preflight()
}

fn script(dir: &Path, name: &str, body: &str) -> String {
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/usr/bin/env bash\nset -u\n{body}\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    path.to_string_lossy().into_owned()
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
    let checks = port()
        .preflight(&config(vec![create, "--state-dir".into(), "/s".into()]))
        .unwrap();
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
    let error = port().preflight(&config(vec![create.clone()])).unwrap_err();
    assert!(error.contains(&create), "{error}");
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
fn a_silent_script_and_a_missing_one_are_errors_not_healthy_reports() {
    let dir = tempfile::TempDir::new().unwrap();
    let silent = script(dir.path(), "silent.sh", "exit 0");
    let error = port().preflight(&config(vec![silent])).unwrap_err();
    assert!(error.contains("reported no checks"), "{error}");
    let error = port()
        .preflight(&config(vec!["/definitely/not/create".into()]))
        .unwrap_err();
    assert!(error.contains("failed to invoke"), "{error}");
    let error = port().preflight(&config(vec![])).unwrap_err();
    assert!(!error.is_empty());
}
