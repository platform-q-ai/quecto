use super::*;
use crate::application::environments::ports::ContainerRuntimeInventory;

fn script(dir: &Path, name: &str, body: &str) -> Vec<String> {
    let path = dir.join(name);
    std::fs::write(&path, format!("{body}\n")).unwrap();
    vec!["bash".to_string(), path.to_string_lossy().into_owned()]
}

fn config(inspect: Vec<String>, cleanup: Vec<String>) -> DiagnosableContainerConfig {
    DiagnosableContainerConfig {
        name: "official".into(),
        create: vec![],
        inspect,
        cleanup,
        diagnostics: vec![],
    }
}

#[test]
fn listing_lines_are_parsed_blank_lines_skipped_and_malformed_lines_refuse_the_listing() {
    let listed = parse_listing(
        b"{\"environment_id\":\"env-a\",\"container\":\"quecto-env-a\",\"status\":\"running\"}\n\n{\"environment_id\":\"env-b\",\"container\":\"quecto-env-b\",\"status\":\"dead\"}\n",
    )
    .unwrap();
    assert_eq!(
        listed,
        vec![
            RuntimeContainer {
                environment_id: "env-a".into(),
                container: "quecto-env-a".into(),
                running: true
            },
            RuntimeContainer {
                environment_id: "env-b".into(),
                container: "quecto-env-b".into(),
                running: false
            },
        ]
    );
    assert!(parse_listing(b"").unwrap().is_empty());
    let error = parse_listing(b"{\"environment_id\":\"env-a\"}\n").unwrap_err();
    assert!(error.contains("malformed line 1"), "{error}");
    let error =
        parse_listing(b"{\"environment_id\":\"\",\"container\":\"x\",\"status\":\"dead\"}\n")
            .unwrap_err();
    assert!(error.contains("without an environment_id"), "{error}");
    let error = parse_listing(b"not json\n").unwrap_err();
    assert!(error.contains("malformed line 1"), "{error}");
}

#[test]
fn containers_runs_the_inspect_with_the_list_flag_and_no_environment_id() {
    let dir = tempfile::TempDir::new().unwrap();
    let inspect = script(
        dir.path(),
        "inspect.sh",
        r#"[ "${@: -1}" = --list ] || { echo 'no --list' >&2; exit 2; }
[ -z "${QUECTO_CONTAINER_ENVIRONMENT_ID:-}" ] || exit 3
printf '{"environment_id":"env-a","container":"quecto-env-a","status":"running"}\n'"#,
    );
    // SAFETY: set only for this call and removed right after; the adapter strips it before the script runs, which the script asserts.
    unsafe { std::env::set_var("QUECTO_CONTAINER_ENVIRONMENT_ID", "stale") };
    let listed = ScriptInventory
        .containers(&config(inspect, vec!["true".into()]))
        .unwrap();
    // SAFETY: restores the process environment the test found.
    unsafe { std::env::remove_var("QUECTO_CONTAINER_ENVIRONMENT_ID") };
    assert_eq!(listed.len(), 1);
    assert!(listed[0].running);
}

#[test]
fn a_script_set_without_list_or_inspect_is_an_unavailable_inventory() {
    let dir = tempfile::TempDir::new().unwrap();
    let legacy = script(
        dir.path(),
        "legacy.sh",
        "echo 'unknown argument: --list' >&2; exit 1",
    );
    let error = ScriptInventory
        .containers(&config(legacy, vec!["true".into()]))
        .unwrap_err();
    assert!(error.contains("unknown argument: --list"), "{error}");
    assert!(error.contains("cannot serve the collector"), "{error}");
    let error = ScriptInventory
        .containers(&config(vec![], vec!["true".into()]))
        .unwrap_err();
    assert!(error.contains("no inspect script"), "{error}");
}

#[test]
fn environment_dirs_are_the_env_prefixed_directories_with_their_container_file() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().join("state");
    std::fs::create_dir_all(root.join("env-b/workspace")).unwrap();
    std::fs::write(root.join("env-b/container"), "quecto-env-b\n").unwrap();
    std::fs::create_dir_all(root.join("env-a")).unwrap();
    std::fs::create_dir_all(root.join("other")).unwrap();
    std::fs::write(root.join("creates.log"), "env-a\n").unwrap();
    std::fs::write(root.join("env-file"), "").unwrap();
    let dirs = ScriptInventory.environment_dirs(&root).unwrap();
    assert_eq!(
        dirs,
        vec![
            EnvironmentStateDir {
                path: root.join("env-a"),
                environment_id: "env-a".into(),
                container: None
            },
            EnvironmentStateDir {
                path: root.join("env-b"),
                environment_id: "env-b".into(),
                container: Some("quecto-env-b".into())
            },
        ]
    );
    assert!(
        ScriptInventory
            .environment_dirs(&root.join("missing"))
            .unwrap()
            .is_empty()
    );
    std::fs::write(dir.path().join("file"), "").unwrap();
    assert!(
        ScriptInventory
            .environment_dirs(&dir.path().join("file"))
            .is_err()
    );
}

#[test]
fn remove_runs_the_configs_cleanup_for_a_well_formed_id_only() {
    let dir = tempfile::TempDir::new().unwrap();
    let log = dir.path().join("cleanup.log");
    let cleanup = script(
        dir.path(),
        "cleanup.sh",
        &format!(
            "printf '%s\\n' \"$QUECTO_CONTAINER_ENVIRONMENT_ID\" >> '{}'",
            log.display()
        ),
    );
    let cleaning = config(vec!["true".into()], cleanup);
    ScriptInventory.remove(&cleaning, "env-abc_1").unwrap();
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "env-abc_1\n");
    for bad in ["", "../x", "env a", "env-x;rm"] {
        let error = ScriptInventory.remove(&cleaning, bad).unwrap_err();
        assert!(error.contains("not an environment id"), "{bad:?}: {error}");
    }
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "env-abc_1\n");
    let failing = script(dir.path(), "bad.sh", "echo 'escapes root' >&2; exit 1");
    let error = ScriptInventory
        .remove(&config(vec!["true".into()], failing), "env-z")
        .unwrap_err();
    assert!(error.contains("escapes root"), "{error}");
}

#[test]
fn a_listing_that_hangs_is_bounded_and_reported() {
    let dir = tempfile::TempDir::new().unwrap();
    let inspect = script(dir.path(), "slow.sh", "printf 'x' >&2; exit 3");
    let error = ScriptInventory
        .containers(&config(inspect, vec!["true".into()]))
        .unwrap_err();
    assert!(error.contains("exited with exit status: 3"), "{error}");
    let missing = config(vec!["/definitely/not/here".into()], vec!["true".into()]);
    let error = ScriptInventory.containers(&missing).unwrap_err();
    assert!(error.contains("inspect --list"), "{error}");
}
