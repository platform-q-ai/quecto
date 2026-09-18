//! Contract for the `ContainerRuntimeInventory` port (#2024 S4d), proven
//! on the production adapter over a fake runtime CLI: `containers` lists
//! every container the runtime labels with an environment id, with its
//! name, label and whether it runs; `remove_container` removes one by name
//! and refuses anything that is not an environment container name;
//! `environment_dirs` lists the `env-*` directories directly under a root
//! with the container each names (a missing root is empty, not an error);
//! `remove_environment_dir` removes one such directory and refuses a path
//! that is not an `env-*` entry directly under the root, or a link.
use std::path::{Path, PathBuf};
use std::sync::Arc;

use quecto::application::environments::dto::{EnvironmentStateDir, RuntimeContainer};
use quecto::application::environments::ports::ContainerRuntimeInventory;
use quecto::infrastructure::processes::containers::runtime_inventory::RuntimeCliInventory;

/// A fake runtime CLI backed by a directory of container files.
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
    [ "$2" = -a ] || exit 2
    for f in "$store"/*; do
      [ -e "$f" ] || continue
      printf '%s\t%s\t%s\n' "$(basename "$f")" "$(sed -n 1p "$f")" "$(sed -n 2p "$f")"
    done ;;
  rm) [ "$2" = -f ] || exit 2; rm -f "$store/$3" ;;
  *) exit 2 ;;
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

fn port(dir: &Path) -> Arc<dyn ContainerRuntimeInventory> {
    Arc::new(RuntimeCliInventory::with_cli(fake_cli(dir)))
}

#[test]
fn containers_lists_labelled_containers_with_name_label_and_liveness() {
    let dir = tempfile::TempDir::new().unwrap();
    let inventory = port(dir.path());
    container(dir.path(), "quecto-env-live", "running", "env-live");
    container(dir.path(), "quecto-env-dead", "exited", "env-dead");
    let mut listed = inventory.containers().unwrap();
    listed.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(
        listed,
        vec![
            RuntimeContainer {
                name: "quecto-env-dead".into(),
                environment_id: Some("env-dead".into()),
                running: false
            },
            RuntimeContainer {
                name: "quecto-env-live".into(),
                environment_id: Some("env-live".into()),
                running: true
            },
        ]
    );
}

#[test]
fn remove_container_removes_by_name_and_refuses_non_environment_names() {
    let dir = tempfile::TempDir::new().unwrap();
    let inventory = port(dir.path());
    container(dir.path(), "quecto-env-dead", "exited", "env-dead");
    inventory.remove_container("quecto-env-dead").unwrap();
    assert!(inventory.containers().unwrap().is_empty());
    for name in ["postgres", "quecto-env-x y", "quecto-env-x;rm", ""] {
        assert!(inventory.remove_container(name).is_err(), "{name:?}");
    }
}

#[test]
fn environment_dirs_lists_env_entries_directly_under_the_root_with_their_container() {
    let dir = tempfile::TempDir::new().unwrap();
    let inventory = port(dir.path());
    let root = dir.path().join("state");
    std::fs::create_dir_all(root.join("env-a/workspace")).unwrap();
    std::fs::write(root.join("env-a/container"), "quecto-env-a\n").unwrap();
    std::fs::create_dir_all(root.join("env-b")).unwrap();
    std::fs::create_dir_all(root.join("not-env")).unwrap();
    std::fs::write(root.join("creates.log"), "").unwrap();
    assert_eq!(
        inventory.environment_dirs(&root).unwrap(),
        vec![
            EnvironmentStateDir {
                path: root.join("env-a"),
                environment_id: "env-a".into(),
                container: Some("quecto-env-a".into())
            },
            EnvironmentStateDir {
                path: root.join("env-b"),
                environment_id: "env-b".into(),
                container: None
            },
        ]
    );
    assert!(
        inventory
            .environment_dirs(&root.join("absent"))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn remove_environment_dir_is_fenced_to_env_entries_directly_under_the_root() {
    let dir = tempfile::TempDir::new().unwrap();
    let inventory = port(dir.path());
    let root = dir.path().join("state");
    std::fs::create_dir_all(root.join("env-a/workspace")).unwrap();
    std::fs::create_dir_all(root.join("keep")).unwrap();
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(outside.join("env-z")).unwrap();
    assert!(
        inventory
            .remove_environment_dir(&root, &root.join("keep"))
            .is_err()
    );
    assert!(
        inventory
            .remove_environment_dir(&root, &outside.join("env-z"))
            .is_err()
    );
    assert!(
        inventory
            .remove_environment_dir(&root, &root.join("env-a/workspace"))
            .is_err()
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, root.join("env-link")).unwrap();
        assert!(
            inventory
                .remove_environment_dir(&root, &root.join("env-link"))
                .is_err()
        );
        assert!(outside.join("env-z").exists());
    }
    inventory
        .remove_environment_dir(&root, &root.join("env-a"))
        .unwrap();
    assert!(!root.join("env-a").exists());
    inventory
        .remove_environment_dir(&root, &root.join("env-a"))
        .unwrap();
    assert!(root.join("keep").exists() && outside.join("env-z").exists());
}
