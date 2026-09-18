//! Contract for the `ContainerScriptIntegrity` port (#2024 S4e): a script
//! at `<project>/.quecto/containers/standard/<asset>` is judged against
//! the bytes this binary embeds — intact, differing, missing, or refused
//! when a symbolic link stands in its place or on the way — exactly as
//! `quecto container status` judges the same file; any other path is not
//! the bundle's and gets no verdict.
use std::path::Path;

use quecto::application::environments::dto::STANDARD_CONTAINER_DIR;
use quecto::application::subagents::dto::StandardScriptVerdict;
use quecto::composition::container_configs::build_container_script_integrity;
use quecto::composition::standard_container::build_container_asset_store;

fn materialised() -> (tempfile::TempDir, std::path::PathBuf) {
    let project = tempfile::tempdir().unwrap();
    let dir = project.path().join(STANDARD_CONTAINER_DIR);
    let store = build_container_asset_store();
    for asset in store.catalogue().assets {
        store.materialise(project.path(), &dir, &asset).unwrap();
    }
    (project, dir)
}

#[test]
fn a_materialised_script_is_intact_until_its_bytes_change() {
    let (_project, dir) = materialised();
    let integrity = build_container_script_integrity();
    for script in [
        "scripts/create.sh",
        "scripts/exec.sh",
        "scripts/inspect.sh",
        "scripts/kill.sh",
    ] {
        assert_eq!(
            integrity.verify(&dir.join(script)),
            StandardScriptVerdict::Intact,
            "{script}"
        );
    }
    let create = dir.join("scripts/create.sh");
    let mut bytes = std::fs::read(&create).unwrap();
    bytes.extend_from_slice(b"\necho tampered\n");
    std::fs::write(&create, bytes).unwrap();
    assert_eq!(integrity.verify(&create), StandardScriptVerdict::Differs);
    // The other scripts keep their verdict: one file, one verdict.
    assert_eq!(
        integrity.verify(&dir.join("scripts/exec.sh")),
        StandardScriptVerdict::Intact
    );
}

#[test]
fn a_missing_or_symlinked_script_is_named_not_followed() {
    let (_project, dir) = materialised();
    let integrity = build_container_script_integrity();
    let inspect = dir.join("scripts/inspect.sh");
    std::fs::remove_file(&inspect).unwrap();
    assert_eq!(integrity.verify(&inspect), StandardScriptVerdict::Missing);
    #[cfg(unix)]
    {
        // A link to an intact script in the asset's place is refused: the
        // bytes behind it are not what the path promises.
        std::os::unix::fs::symlink(dir.join("scripts/exec.sh"), &inspect).unwrap();
        match integrity.verify(&inspect) {
            StandardScriptVerdict::Refused(reason) => {
                assert!(reason.contains("symbolic link"), "{reason}")
            }
            other => panic!("expected Refused, got {other:?}"),
        }
    }
}

#[test]
fn a_script_outside_the_bundle_gets_no_verdict() {
    let (_project, dir) = materialised();
    let integrity = build_container_script_integrity();
    assert_eq!(
        integrity.verify(Path::new("/bin/true")),
        StandardScriptVerdict::NotStandard
    );
    assert_eq!(
        integrity.verify(&dir.join("scripts/mine.sh")),
        StandardScriptVerdict::NotStandard
    );
    assert_eq!(
        integrity.verify(Path::new(".quecto/containers/standard/scripts/create.sh")),
        StandardScriptVerdict::NotStandard,
        "a relative path is not a materialised asset"
    );
}
