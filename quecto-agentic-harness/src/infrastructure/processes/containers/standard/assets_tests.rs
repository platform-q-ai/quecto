use std::path::Path;

use super::{EmbeddedStandardAssets, STANDARD_ASSET_VERSION};
use crate::application::environments::dto::{AssetOutcome, AssetState};
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

#[test]
fn materialise_writes_missing_files_with_their_mode_and_never_replaces_existing_ones() {
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
    assert_eq!(std::fs::read(&path).unwrap(), create.contents);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }
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
    let containerfile = &catalogue.assets[0];
    store
        .materialise(dir.path(), dir.path(), containerfile)
        .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(dir.path().join("Containerfile"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o644
        );
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
