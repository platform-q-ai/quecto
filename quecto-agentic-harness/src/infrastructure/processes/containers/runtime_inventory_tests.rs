use super::*;
use crate::application::environments::ports::ContainerRuntimeInventory;

/// A fake runtime CLI backed by a directory: `ps -a` lists one file per
/// container (`<name>` holding `running`/`exited` and the label), `rm -f`
/// deletes the file.
fn fake_cli(dir: &Path) -> PathBuf {
    let store = dir.join("containers");
    std::fs::create_dir_all(&store).unwrap();
    let cli = dir.join("podman");
    std::fs::write(
        &cli,
        format!(
            r#"#!/usr/bin/env bash
store='{store}'
case "$1" in
  ps)
    for f in "$store"/*; do
      [ -e "$f" ] || continue
      name="$(basename "$f")"
      state="$(sed -n 1p "$f")"; label="$(sed -n 2p "$f")"
      printf '%s\t%s\t%s\n' "$name" "$state" "$label"
    done ;;
  rm) rm -f "$store/$3" ;;
  *) echo "unsupported: $*" >&2; exit 2 ;;
esac"#,
            store = store.display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    cli
}

fn container(dir: &Path, name: &str, state: &str, label: &str) {
    std::fs::write(
        dir.join("containers").join(name),
        format!("{state}\n{label}\n"),
    )
    .unwrap();
}

#[test]
fn ps_lines_are_parsed_into_containers_with_their_label() {
    assert_eq!(
        parse_ps_line("quecto-env-a\trunning\tenv-a"),
        Some(RuntimeContainer {
            name: "quecto-env-a".into(),
            environment_id: Some("env-a".into()),
            running: true
        })
    );
    assert_eq!(
        parse_ps_line("quecto-env-b\tExited\t"),
        Some(RuntimeContainer {
            name: "quecto-env-b".into(),
            environment_id: None,
            running: false
        })
    );
    assert_eq!(parse_ps_line(""), None);
}

#[test]
fn the_runtime_is_asked_for_labelled_containers_and_removes_only_environment_ones() {
    let dir = tempfile::TempDir::new().unwrap();
    let cli = fake_cli(dir.path());
    container(dir.path(), "quecto-env-a", "running", "env-a");
    container(dir.path(), "quecto-env-b", "exited", "env-b");
    let inventory = RuntimeCliInventory::with_cli(cli);
    let mut listed = inventory.containers().unwrap();
    listed.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(listed.len(), 2);
    assert!(listed[0].running && !listed[1].running);
    inventory.remove_container("quecto-env-b").unwrap();
    assert_eq!(inventory.containers().unwrap().len(), 1);
    let error = inventory.remove_container("postgres").unwrap_err();
    assert!(
        error.contains("not an environment container name"),
        "{error}"
    );
    let error = inventory
        .remove_container("quecto-env-x; rm -rf /")
        .unwrap_err();
    assert!(
        error.contains("not an environment container name"),
        "{error}"
    );
}

#[test]
fn no_runtime_on_path_is_an_unavailable_inventory_not_an_empty_one() {
    let inventory = RuntimeCliInventory { cli: None };
    let error = inventory.containers().unwrap_err();
    assert!(error.contains("no container runtime on PATH"), "{error}");
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
    let inventory = RuntimeCliInventory { cli: None };
    let dirs = inventory.environment_dirs(&root).unwrap();
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
        inventory
            .environment_dirs(&root.join("missing"))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn removal_is_fenced_to_env_directories_directly_under_the_root() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().join("state");
    std::fs::create_dir_all(root.join("env-a/workspace/repo")).unwrap();
    std::fs::create_dir_all(root.join("other")).unwrap();
    let elsewhere = dir.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let inventory = RuntimeCliInventory { cli: None };
    let error = inventory
        .remove_environment_dir(&root, &root.join("other"))
        .unwrap_err();
    assert!(error.contains("not an environment directory"), "{error}");
    let error = inventory
        .remove_environment_dir(&root, &elsewhere.join("env-z"))
        .unwrap_err();
    assert!(error.contains("not an environment directory"), "{error}");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&elsewhere, root.join("env-link")).unwrap();
        let error = inventory
            .remove_environment_dir(&root, &root.join("env-link"))
            .unwrap_err();
        assert!(error.contains("symbolic link"), "{error}");
        assert!(elsewhere.exists());
    }
    inventory
        .remove_environment_dir(&root, &root.join("env-a"))
        .unwrap();
    assert!(!root.join("env-a").exists());
    // Already gone is done.
    inventory
        .remove_environment_dir(&root, &root.join("env-a"))
        .unwrap();
    assert!(root.join("other").exists());
}
