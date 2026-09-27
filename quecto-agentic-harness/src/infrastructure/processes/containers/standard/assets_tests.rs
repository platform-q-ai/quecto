use std::path::Path;

use super::{EmbeddedStandardAssets, STANDARD_ASSET_VERSION};
use crate::application::environments::dto::{AssetOutcome, AssetOwnership, AssetState};
use crate::application::environments::ports::ContainerAssetStore;

fn workspace_file(relative: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join(relative),
    )
    .unwrap()
}

/// Only the Containerfile is the project's. A script marked `Project` would
/// read as `yours` in status and never be restored by `--refresh`.
#[test]
fn only_the_containerfile_is_project_owned() {
    let owners: Vec<(String, AssetOwnership)> = EmbeddedStandardAssets
        .catalogue()
        .assets
        .into_iter()
        .map(|asset| (asset.path, asset.ownership))
        .collect();
    assert_eq!(
        owners,
        [
            ("Containerfile".to_string(), AssetOwnership::Project),
            ("scripts/create.sh".to_string(), AssetOwnership::Bundle),
            ("scripts/exec.sh".to_string(), AssetOwnership::Bundle),
            ("scripts/inspect.sh".to_string(), AssetOwnership::Bundle),
            ("scripts/kill.sh".to_string(), AssetOwnership::Bundle),
        ]
    );
}

#[test]
fn the_catalogue_is_the_official_adapter_set_plus_the_containerfile() {
    let catalogue = EmbeddedStandardAssets.catalogue();
    assert_eq!(catalogue.version, STANDARD_ASSET_VERSION);
    let paths: Vec<&str> = catalogue.assets.iter().map(|a| a.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "Containerfile",
            "scripts/create.sh",
            "scripts/exec.sh",
            "scripts/inspect.sh",
            "scripts/kill.sh"
        ]
    );
    for (asset, source) in catalogue.assets.iter().skip(1).zip([
        "scripts/container-runtime/docker/create.sh",
        "scripts/container-runtime/docker/exec.sh",
        "scripts/container-runtime/docker/inspect.sh",
        "scripts/container-runtime/docker/kill.sh",
    ]) {
        assert_eq!(
            asset.contents,
            workspace_file(source).into_bytes(),
            "{source}"
        );
        assert!(asset.executable);
    }
    assert!(!catalogue.assets[0].executable);
    let containerfile = String::from_utf8(catalogue.assets[0].contents.clone()).unwrap();
    assert!(containerfile.contains("FROM "), "{containerfile}");
    assert!(containerfile.contains("ENTRYPOINT []"), "{containerfile}");
}

#[cfg(unix)]
#[test]
fn materialise_writes_missing_files_with_their_mode_and_never_replaces_existing_ones() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::TempDir::new().unwrap();
    let store = EmbeddedStandardAssets;
    let catalogue = store.catalogue();
    let create = &catalogue.assets[1];
    assert_eq!(
        store.observe(dir.path(), dir.path(), create).unwrap(),
        AssetState::Missing
    );
    assert_eq!(
        store.materialise(dir.path(), dir.path(), create).unwrap(),
        AssetOutcome::Written
    );
    let path = dir.path().join("scripts/create.sh");
    // A regular file in its own right (never a link), judged without
    // following anything.
    let placed = std::fs::symlink_metadata(&path).unwrap();
    assert!(placed.file_type().is_file(), "{:?}", placed.file_type());
    assert_eq!(std::fs::read(&path).unwrap(), create.contents);
    assert_eq!(placed.permissions().mode() & 0o777, 0o755);
    assert_eq!(
        store.observe(dir.path(), dir.path(), create).unwrap(),
        AssetState::Identical
    );
    assert_eq!(
        store.materialise(dir.path(), dir.path(), create).unwrap(),
        AssetOutcome::KeptIdentical
    );
    std::fs::write(&path, "edited").unwrap();
    assert_eq!(
        store.observe(dir.path(), dir.path(), create).unwrap(),
        AssetState::Differs
    );
    assert_eq!(
        store.materialise(dir.path(), dir.path(), create).unwrap(),
        AssetOutcome::KeptDiffering
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "edited");
    let leftovers: Vec<_> = std::fs::read_dir(dir.path().join("scripts"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(leftovers, ["create.sh"], "no temporary file remains");
    // A refresh replaces the differing file with the embedded bytes and
    // mode; an identical one is kept; a missing one is written.
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(
        store.refresh(dir.path(), dir.path(), create).unwrap(),
        AssetOutcome::Refreshed
    );
    let refreshed = std::fs::symlink_metadata(&path).unwrap();
    assert!(refreshed.file_type().is_file());
    assert_eq!(std::fs::read(&path).unwrap(), create.contents);
    assert_eq!(refreshed.permissions().mode() & 0o777, 0o755);
    assert_eq!(
        store.refresh(dir.path(), dir.path(), create).unwrap(),
        AssetOutcome::KeptIdentical
    );
    std::fs::remove_file(&path).unwrap();
    assert_eq!(
        store.refresh(dir.path(), dir.path(), create).unwrap(),
        AssetOutcome::Written
    );
    let leftovers: Vec<_> = std::fs::read_dir(dir.path().join("scripts"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(leftovers, ["create.sh"], "no temporary file remains");
    let containerfile = &catalogue.assets[0];
    store
        .materialise(dir.path(), dir.path(), containerfile)
        .unwrap();
    {
        let containerfile = std::fs::symlink_metadata(dir.path().join("Containerfile")).unwrap();
        assert!(containerfile.file_type().is_file());
        assert_eq!(containerfile.permissions().mode() & 0o777, 0o644);
    }
}

#[cfg(unix)]
#[test]
fn a_symbolic_link_in_an_assets_place_is_refused_not_followed() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = EmbeddedStandardAssets;
    let containerfile = &store.catalogue().assets[0];
    let target = dir.path().join("elsewhere");
    std::fs::write(&target, "x").unwrap();
    std::os::unix::fs::symlink(&target, dir.path().join("Containerfile")).unwrap();
    let error = store
        .observe(dir.path(), dir.path(), containerfile)
        .unwrap_err();
    assert!(error.contains("symbolic link"), "{error}");
    let error = store
        .materialise(dir.path(), dir.path(), containerfile)
        .unwrap_err();
    assert!(error.contains("symbolic link"), "{error}");
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "x");
}

#[cfg(unix)]
#[test]
fn a_symbolic_link_in_a_directorys_place_is_refused_not_followed() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = EmbeddedStandardAssets;
    let create = &store.catalogue().assets[1];
    let elsewhere = dir.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let bundle = dir.path().join("bundle");
    std::fs::create_dir_all(&bundle).unwrap();
    std::os::unix::fs::symlink(&elsewhere, bundle.join("scripts")).unwrap();
    let error = store.materialise(dir.path(), &bundle, create).unwrap_err();
    assert!(error.contains("symbolic link"), "{error}");
    assert!(std::fs::read_dir(&elsewhere).unwrap().next().is_none());
    // The bundle directory itself being a link is refused too.
    let linked_bundle = dir.path().join("linked-bundle");
    std::os::unix::fs::symlink(&elsewhere, &linked_bundle).unwrap();
    let error = store
        .materialise(dir.path(), &linked_bundle, create)
        .unwrap_err();
    assert!(error.contains("symbolic link"), "{error}");
    assert!(std::fs::read_dir(&elsewhere).unwrap().next().is_none());
}

#[cfg(unix)]
#[test]
fn a_symbolic_link_between_the_root_and_the_bundle_is_refused() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = EmbeddedStandardAssets;
    let create = &store.catalogue().assets[1];
    let elsewhere = dir.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir_all(project.join(".quecto")).unwrap();
    std::os::unix::fs::symlink(&elsewhere, project.join(".quecto/containers")).unwrap();
    let bundle = project.join(".quecto/containers/standard");
    let error = store.observe(&project, &bundle, create).unwrap_err();
    assert!(error.contains("containers is a symbolic link"), "{error}");
    let error = store.materialise(&project, &bundle, create).unwrap_err();
    assert!(error.contains("containers is a symbolic link"), "{error}");
    assert!(std::fs::read_dir(&elsewhere).unwrap().next().is_none());
}

#[cfg(unix)]
#[test]
fn a_directory_in_an_assets_place_and_a_file_in_a_directorys_place_are_refused() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = EmbeddedStandardAssets;
    let catalogue = store.catalogue();
    let containerfile = &catalogue.assets[0];
    let create = &catalogue.assets[1];
    // A directory where the Containerfile goes: not a regular file, so
    // neither judged nor written.
    std::fs::create_dir_all(dir.path().join("Containerfile")).unwrap();
    let error = store
        .observe(dir.path(), dir.path(), containerfile)
        .unwrap_err();
    assert!(
        error.contains("exists but is not a regular file"),
        "{error}"
    );
    let error = store
        .materialise(dir.path(), dir.path(), containerfile)
        .unwrap_err();
    assert!(
        error.contains("exists but is not a regular file"),
        "{error}"
    );
    assert!(dir.path().join("Containerfile").is_dir());
    // A regular file where `scripts/` goes: the destination below it
    // cannot even be judged (ENOTDIR), and the failure names it.
    std::fs::write(dir.path().join("scripts"), "in the way").unwrap();
    let error = store
        .materialise(dir.path(), dir.path(), create)
        .unwrap_err();
    assert!(
        error.contains(&format!(
            "cannot stat {}",
            dir.path().join("scripts/create.sh").display()
        )),
        "{error}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("scripts")).unwrap(),
        "in the way"
    );
}

#[cfg(unix)]
#[test]
fn an_unreadable_asset_is_an_error_that_names_the_file() {
    use std::os::unix::fs::PermissionsExt;
    // Root reads anything: the check has nothing to observe there.
    // SAFETY: geteuid has no preconditions and cannot fail.
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let dir = tempfile::TempDir::new().unwrap();
    let store = EmbeddedStandardAssets;
    let containerfile = &store.catalogue().assets[0];
    let path = dir.path().join("Containerfile");
    std::fs::write(&path, "x").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
    let error = store
        .observe(dir.path(), dir.path(), containerfile)
        .unwrap_err();
    assert!(
        error.contains(&format!("cannot read {}", path.display())),
        "{error}"
    );
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
}

#[cfg(unix)]
#[test]
fn a_bundle_directory_that_cannot_be_written_is_an_error_that_names_it() {
    use std::os::unix::fs::PermissionsExt;
    // Root writes anywhere: the check has nothing to observe there.
    // SAFETY: geteuid has no preconditions and cannot fail.
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let dir = tempfile::TempDir::new().unwrap();
    let store = EmbeddedStandardAssets;
    let containerfile = &store.catalogue().assets[0];
    let bundle = dir.path().join("bundle");
    std::fs::create_dir_all(&bundle).unwrap();
    std::fs::set_permissions(&bundle, std::fs::Permissions::from_mode(0o555)).unwrap();
    let error = store
        .materialise(dir.path(), &bundle, containerfile)
        .unwrap_err();
    assert!(
        error.contains(&format!(
            "cannot create {}/.quecto-asset-",
            bundle.display()
        )),
        "{error}"
    );
    assert!(error.contains("Permission denied"), "{error}");
    std::fs::set_permissions(&bundle, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(std::fs::read_dir(&bundle).unwrap().next().is_none());
}

/// #2232 review: a placed script is exec'able at once, from a process whose
/// other threads fork — `place` holds no writable descriptor on it, so a
/// concurrent fork cannot make the exec "Text file busy". With the former
/// in-process tempfile write, a few percent of these execs were refused.
#[test]
fn a_placed_script_runs_at_once_while_other_threads_fork() {
    use crate::infrastructure::test_support::executable::{exec_is_busy, while_forking};
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::TempDir::new().unwrap();
    let store = EmbeddedStandardAssets;
    let mut asset = store
        .catalogue()
        .assets
        .into_iter()
        .find(|asset| asset.executable)
        .expect("the bundle ships an executable script");
    asset.contents = b"#!/bin/sh\nexit 0\n".to_vec();
    let busy = while_forking(2, || {
        (0..300)
            .filter(|index| {
                let bundle = dir.path().join(format!("bundle-{index}"));
                assert_eq!(
                    store.materialise(dir.path(), &bundle, &asset).unwrap(),
                    AssetOutcome::Written
                );
                let script = bundle.join(&asset.path);
                assert_eq!(
                    std::fs::metadata(&script).unwrap().permissions().mode() & 0o777,
                    0o755
                );
                exec_is_busy(&script)
            })
            .count()
    });
    assert_eq!(busy, 0, "placed scripts refused with ETXTBSY");
}

/// Plants what the `persist-probe` test's current case needs in the
/// windows around the no-clobber link of a new asset.
fn plant(destination: &Path, step: &str) {
    if !destination.to_string_lossy().contains("persist-probe") {
        return;
    }
    let case = std::fs::read_to_string(destination.with_file_name("case")).unwrap();
    match (case.as_str(), step) {
        ("identical", "persist") => std::fs::write(destination, b"ours").unwrap(),
        ("differs", "persist") => std::fs::write(destination, b"theirs").unwrap(),
        ("directory", "persist") => std::fs::create_dir(destination).unwrap(),
        ("vanished", "persist") => std::fs::write(destination, b"ours").unwrap(),
        ("vanished", "kept") => std::fs::remove_file(destination).unwrap(),
        _ => {}
    }
}

/// #2232 review round 3: a name taken while a new asset was written is
/// judged on what took it, every state by name; a non-file or a vanished
/// name is an error, never reported as kept.
#[test]
fn a_name_taken_during_the_write_is_reported_by_what_took_it() {
    let _serial = super::PLACE_HOOK_SERIAL
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let store = EmbeddedStandardAssets;
    let mut asset = store.catalogue().assets[0].clone();
    asset.path = "planted".into();
    asset.contents = b"ours".to_vec();
    *super::PLACE_HOOK.lock().unwrap() = Some(plant);
    let outcomes: Vec<_> = ["identical", "differs", "directory", "vanished"]
        .into_iter()
        .map(|case| {
            let root = tempfile::Builder::new()
                .prefix("persist-probe")
                .tempdir()
                .unwrap();
            let bundle = root.path().join("bundle");
            std::fs::create_dir_all(&bundle).unwrap();
            std::fs::write(bundle.join("case"), case).unwrap();
            (case, store.materialise(root.path(), &bundle, &asset))
        })
        .collect();
    *super::PLACE_HOOK.lock().unwrap() = None;
    for (case, outcome) in outcomes {
        match case {
            "identical" => assert_eq!(outcome, Ok(AssetOutcome::KeptIdentical), "{case}"),
            "differs" => assert_eq!(outcome, Ok(AssetOutcome::KeptDiffering), "{case}"),
            "directory" => assert!(
                outcome
                    .as_ref()
                    .is_err_and(|e| e.contains("not a regular file")),
                "{case}: {outcome:?}"
            ),
            _ => assert!(
                outcome.as_ref().is_err_and(|e| e.contains("try again")),
                "{case}: {outcome:?}"
            ),
        }
    }
}
