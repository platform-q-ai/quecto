//! Contract for the `ContainerRuntimeInventory` port (#2024 S4d), proven
//! on the production script adapter: `containers` asks the config's own
//! `inspect` argv with `--list` and no environment id, and every JSON line
//! it prints is one environment the runtime knows (id, container, whether
//! it runs) — a script set that refuses `--list`, prints a malformed line
//! or has no inspect is an unavailable inventory, never an empty one;
//! `environment_dirs` lists the `env-*` directories directly under a root
//! with the container each names (a missing root is empty, not an error);
//! `remove` runs the config's `cleanup` argv for one well-formed
//! environment id and reports the script's failure in its own words;
//! `canonical_root` is the host's resolved form of a root (a symlink
//! names its target) and a root that cannot be resolved is itself.
use std::path::Path;
use std::sync::Arc;

use quecto::application::environments::dto::{
    DiagnosableContainerConfig, EnvironmentStateDir, RuntimeContainer,
};
use quecto::application::environments::ports::ContainerRuntimeInventory;
use quecto::composition::environments::build_container_runtime_inventory;

fn port() -> Arc<dyn ContainerRuntimeInventory> {
    build_container_runtime_inventory()
}

fn script(dir: &Path, name: &str, body: &str) -> Vec<String> {
    let path = dir.join(name);
    std::fs::write(&path, format!("{body}\n")).unwrap();
    vec!["bash".to_string(), path.to_string_lossy().into_owned()]
}

fn config(inspect: Vec<String>, cleanup: Vec<String>) -> DiagnosableContainerConfig {
    DiagnosableContainerConfig {
        name: "box".into(),
        create: vec![],
        inspect,
        cleanup,
        diagnostics: vec![],
    }
}

#[test]
fn containers_are_the_inspect_listings_lines_asked_without_an_environment_id() {
    let dir = tempfile::TempDir::new().unwrap();
    let inspect = script(
        dir.path(),
        "inspect.sh",
        r#"[ "${@: -1}" = --list ] || { echo 'inspect needs --list here' >&2; exit 2; }
[ -z "${QUECTO_CONTAINER_ENVIRONMENT_ID:-}" ] || { echo 'listing carries no id' >&2; exit 3; }
printf '{"environment_id":"env-live","container":"quecto-env-live","status":"running"}\n'
printf '{"environment_id":"env-dead","container":"quecto-env-dead","status":"dead"}\n'"#,
    );
    let listed = port().containers(&config(inspect, vec![])).unwrap();
    assert_eq!(
        listed,
        vec![
            RuntimeContainer {
                environment_id: "env-live".into(),
                container: "quecto-env-live".into(),
                running: true
            },
            RuntimeContainer {
                environment_id: "env-dead".into(),
                container: "quecto-env-dead".into(),
                running: false
            },
        ]
    );
}

#[test]
fn a_script_set_that_cannot_list_is_an_unavailable_inventory_never_an_empty_one() {
    let dir = tempfile::TempDir::new().unwrap();
    let legacy = script(
        dir.path(),
        "legacy.sh",
        "echo 'unknown argument: --list' >&2; exit 1",
    );
    let error = port().containers(&config(legacy, vec![])).unwrap_err();
    assert!(error.contains("unknown argument: --list"), "{error}");
    let malformed = script(dir.path(), "odd.sh", "echo 'not json'");
    let error = port().containers(&config(malformed, vec![])).unwrap_err();
    assert!(error.contains("malformed line 1"), "{error}");
    assert!(port().containers(&config(vec![], vec![])).is_err());
}

#[test]
fn environment_dirs_lists_env_entries_directly_under_the_root_with_their_container() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().join("state");
    std::fs::create_dir_all(root.join("env-a/workspace")).unwrap();
    std::fs::write(root.join("env-a/container"), "quecto-env-a\n").unwrap();
    std::fs::create_dir_all(root.join("env-b")).unwrap();
    std::fs::create_dir_all(root.join("not-env")).unwrap();
    std::fs::write(root.join("creates.log"), "").unwrap();
    let mut listed = port().environment_dirs(&root).unwrap();
    assert!(
        listed.iter().all(|dir| dir.age_secs.is_some()),
        "{listed:?}"
    );
    for dir in &mut listed {
        dir.age_secs = None;
    }
    assert_eq!(
        listed,
        vec![
            EnvironmentStateDir {
                path: root.join("env-a"),
                environment_id: "env-a".into(),
                container: Some("quecto-env-a".into()),
                age_secs: None,
            },
            EnvironmentStateDir {
                path: root.join("env-b"),
                environment_id: "env-b".into(),
                container: None,
                age_secs: None,
            },
        ]
    );
    assert!(
        port()
            .environment_dirs(&root.join("absent"))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn remove_runs_the_configs_cleanup_for_one_well_formed_id_and_reports_its_failure() {
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
    let cleaning = config(vec![], cleanup);
    port().remove(&cleaning, "env-one").unwrap();
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "env-one\n");
    for bad in ["", "../env-one", "env one", "env-one;rm -rf /"] {
        assert!(port().remove(&cleaning, bad).is_err(), "{bad:?}");
    }
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "env-one\n");
    let failing = script(
        dir.path(),
        "bad.sh",
        "echo 'environment escapes the state root' >&2; exit 1",
    );
    let error = port()
        .remove(&config(vec![], failing), "env-two")
        .unwrap_err();
    assert!(error.contains("escapes the state root"), "{error}");
}

#[test]
fn inspect_asks_the_configs_inspect_for_one_well_formed_id() {
    use quecto::application::environments::dto::EnvironmentLiveness;
    let dir = tempfile::TempDir::new().unwrap();
    let inspect = script(
        dir.path(),
        "inspect.sh",
        r#"[ "${@: -1}" != --list ] || exit 9
case "$QUECTO_CONTAINER_ENVIRONMENT_ID" in
  env-live) printf '{"status":"running","metadata":{}}' ;;
  env-gone) printf '{"status":"dead","metadata":{}}' ;;
  *) echo 'unknown' >&2; exit 1 ;;
esac"#,
    );
    let inspecting = config(inspect, vec![]);
    assert_eq!(
        port().inspect(&inspecting, "env-live"),
        EnvironmentLiveness::Running
    );
    assert_eq!(
        port().inspect(&inspecting, "env-gone"),
        EnvironmentLiveness::Gone
    );
    assert!(matches!(
        port().inspect(&inspecting, "env-other"),
        EnvironmentLiveness::Unknown(reason) if reason.contains("unknown")
    ));
    assert!(matches!(
        port().inspect(&inspecting, "../env-live"),
        EnvironmentLiveness::Unknown(reason) if reason.contains("not an environment id")
    ));
}

/// Round 3 M1 (#2033): the collector compares a record's root with the
/// config's canonically, so a state dir reached through a symlink is the
/// same root — and a root that does not exist resolves to itself.
#[test]
fn canonical_root_resolves_a_symlink_and_leaves_a_missing_root_as_it_is() {
    let dir = tempfile::TempDir::new().unwrap();
    let real = dir.path().join("state");
    std::fs::create_dir_all(&real).unwrap();
    let link = dir.path().join("state-link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let port = port();
    assert_eq!(port.canonical_root(&link), port.canonical_root(&real));
    assert_eq!(
        port.canonical_root(&link),
        std::fs::canonicalize(&real).unwrap()
    );
    let missing = dir.path().join("nowhere");
    assert_eq!(port.canonical_root(&missing), missing);
}
