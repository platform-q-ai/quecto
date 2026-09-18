//! Contract for the `ContainerAssetStore` port (#2024 S4e): the catalogue
//! is the embedded standard bundle — the Containerfile and the official
//! adapter scripts byte-for-byte, scripts executable — with a version;
//! `observe` tells a missing, identical and differing destination apart;
//! `materialise` writes a missing asset whole with its mode and never
//! replaces an existing file (identical or edited); a symbolic link in an
//! asset's place is refused, not followed; nothing is left beside the
//! asset (no temporary file).
use std::path::Path;
use std::sync::Arc;

use quecto::application::environments::dto::{AssetOutcome, AssetState};
use quecto::application::environments::ports::ContainerAssetStore;
use quecto::composition::standard_container::build_container_asset_store;

fn port() -> Arc<dyn ContainerAssetStore> {
    build_container_asset_store()
}

fn workspace_file(relative: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join(relative),
    )
    .unwrap()
}

#[test]
fn the_catalogue_is_the_versioned_embedded_bundle() {
    let catalogue = port().catalogue();
    assert!(catalogue.version >= 1);
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
    for (asset, source) in catalogue.assets.iter().zip([
        "quecto-agentic-harness/assets/standard-container/Containerfile",
        "scripts/container-runtime/docker/create.sh",
        "scripts/container-runtime/docker/exec.sh",
        "scripts/container-runtime/docker/inspect.sh",
        "scripts/container-runtime/docker/kill.sh",
    ]) {
        assert_eq!(asset.contents, workspace_file(source), "{source}");
        assert_eq!(asset.executable, asset.path.starts_with("scripts/"));
    }
    let create = String::from_utf8(catalogue.assets[1].contents.clone()).unwrap();
    assert!(create.contains("--preflight-only"), "S4b contract");
    assert!(
        create.contains("QUECTO_SWARM_CONTAINER=isolated-pid-v1"),
        "S4c contract"
    );
    assert!(create.contains("--pull=never"), "never pulls");
}

#[test]
fn observe_and_materialise_tell_missing_identical_and_differing_apart_and_never_replace() {
    let dir = tempfile::TempDir::new().unwrap();
    let port = port();
    let asset = port.catalogue().assets[1].clone();
    assert_eq!(
        port.observe(dir.path(), &asset).unwrap(),
        AssetState::Missing
    );
    assert_eq!(
        port.materialise(dir.path(), &asset).unwrap(),
        AssetOutcome::Written
    );
    let path = dir.path().join(&asset.path);
    assert_eq!(std::fs::read(&path).unwrap(), asset.contents);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }
    assert_eq!(
        port.observe(dir.path(), &asset).unwrap(),
        AssetState::Identical
    );
    assert_eq!(
        port.materialise(dir.path(), &asset).unwrap(),
        AssetOutcome::KeptIdentical
    );
    std::fs::write(&path, "edited").unwrap();
    assert_eq!(
        port.observe(dir.path(), &asset).unwrap(),
        AssetState::Differs
    );
    assert_eq!(
        port.materialise(dir.path(), &asset).unwrap(),
        AssetOutcome::KeptDiffering
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "edited");
    let names: Vec<_> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["create.sh"]);
}

#[cfg(unix)]
#[test]
fn a_symbolic_link_in_an_assets_place_is_refused() {
    let dir = tempfile::TempDir::new().unwrap();
    let port = port();
    let asset = port.catalogue().assets[0].clone();
    let elsewhere = dir.path().join("elsewhere");
    std::fs::write(&elsewhere, "x").unwrap();
    std::os::unix::fs::symlink(&elsewhere, dir.path().join("Containerfile")).unwrap();
    assert!(
        port.observe(dir.path(), &asset)
            .unwrap_err()
            .contains("symbolic link")
    );
    assert!(
        port.materialise(dir.path(), &asset)
            .unwrap_err()
            .contains("symbolic link")
    );
    assert_eq!(std::fs::read_to_string(&elsewhere).unwrap(), "x");
}
