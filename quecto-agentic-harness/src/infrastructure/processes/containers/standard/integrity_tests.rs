use std::path::Path;

use super::{EmbeddedScriptIntegrity, split_at_bundle};
use crate::application::environments::ports::ContainerAssetStore;
use crate::application::subagents::dto::StandardScriptVerdict;
use crate::application::subagents::ports::ContainerScriptIntegrity;
use crate::infrastructure::processes::containers::standard::assets::EmbeddedStandardAssets;

fn materialised() -> (tempfile::TempDir, std::path::PathBuf) {
    let project = tempfile::tempdir().unwrap();
    let dir = project.path().join(".quecto/containers/standard");
    for asset in EmbeddedStandardAssets.catalogue().assets {
        EmbeddedStandardAssets
            .materialise(project.path(), &dir, &asset)
            .unwrap();
    }
    (project, dir)
}

#[test]
fn splits_a_bundle_path_into_root_and_asset() {
    let (root, relative) = split_at_bundle(Path::new(
        "/p/.quecto/containers/standard/scripts/create.sh",
    ))
    .unwrap();
    assert_eq!(root, Path::new("/p"));
    assert_eq!(relative, Path::new("scripts/create.sh"));
    assert_eq!(
        split_at_bundle(Path::new("/bin/create")),
        Err(StandardScriptVerdict::NotStandard)
    );
    assert_eq!(
        split_at_bundle(Path::new("relative/.quecto/containers/standard/x")),
        Err(StandardScriptVerdict::NotStandard)
    );
}

/// A `..` on the way into or out of the bundle is a path that could name
/// a bundle asset under another spelling: refused outright, never "not
/// the bundle's" (which would run it unjudged).
#[test]
fn a_non_normalised_path_into_the_bundle_is_refused_not_ignored() {
    for path in [
        "/p/.quecto/containers/standard/../standard/scripts/create.sh",
        "/p/.quecto/containers/standard/scripts/../scripts/create.sh",
        "/p/x/../.quecto/containers/standard/scripts/create.sh",
    ] {
        assert_eq!(
            split_at_bundle(Path::new(path)),
            Err(StandardScriptVerdict::Refused(
                "path into the standard bundle is not normalised".into()
            )),
            "{path}"
        );
        assert!(
            matches!(
                EmbeddedScriptIntegrity.verify(Path::new(path)),
                StandardScriptVerdict::Refused(reason) if reason == "path into the standard bundle is not normalised"
            ),
            "{path}"
        );
    }
}

#[test]
fn judges_the_materialised_scripts_as_status_would() {
    let (project, dir) = materialised();
    let create = dir.join("scripts/create.sh");
    assert_eq!(
        EmbeddedScriptIntegrity.verify(&create),
        StandardScriptVerdict::Intact
    );
    let mut bytes = std::fs::read(&create).unwrap();
    bytes.push(b'\n');
    std::fs::write(&create, bytes).unwrap();
    assert_eq!(
        EmbeddedScriptIntegrity.verify(&create),
        StandardScriptVerdict::Differs
    );
    std::fs::remove_file(&create).unwrap();
    assert_eq!(
        EmbeddedScriptIntegrity.verify(&create),
        StandardScriptVerdict::Missing
    );
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(dir.join("scripts/exec.sh"), &create).unwrap();
        assert!(matches!(
            EmbeddedScriptIntegrity.verify(&create),
            StandardScriptVerdict::Refused(reason) if reason.contains("symbolic link")
        ));
    }
    // Not an asset of the bundle, though below its directory.
    assert_eq!(
        EmbeddedScriptIntegrity.verify(&dir.join("scripts/other.sh")),
        StandardScriptVerdict::NotStandard
    );
    assert_eq!(
        EmbeddedScriptIntegrity.verify(Path::new("/bin/true")),
        StandardScriptVerdict::NotStandard
    );
    drop(project);
}
