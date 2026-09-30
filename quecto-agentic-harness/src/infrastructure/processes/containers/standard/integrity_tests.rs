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

/// Exercise the production selector with the checkout's custom adapter, not a
/// fake integrity verdict. Custom adapters must live outside the reserved bundle.
#[test]
fn production_selector_accepts_the_checkout_custom_adapter() {
    use crate::application::subagents::dto::{
        ContainerConfigSource, ContainerConfigsError, ContainerLaunchConfig,
        EffectiveContainerConfigSet, SelectContainerConfigRequest,
    };
    use crate::application::subagents::ports::EffectiveContainerConfigs;
    use crate::application::subagents::use_cases::select_container_config::SelectContainerConfig;
    use std::sync::Arc;

    struct CheckoutConfig(ContainerLaunchConfig);
    impl EffectiveContainerConfigs for CheckoutConfig {
        fn effective_container_configs(
            &self,
            _: &ContainerConfigSource,
        ) -> Result<EffectiveContainerConfigSet, ContainerConfigsError> {
            Ok(EffectiveContainerConfigSet {
                configs: vec![self.0.clone()],
                diagnostics: vec![],
                overlay_withheld: false,
            })
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let custom = root.join(".quecto/containers/standard/create.sh");
    let adapter = if custom.is_file() {
        custom
    } else {
        root.join(".quecto/containers/standard/scripts/create.sh")
    };
    assert!(adapter.is_file(), "checkout custom adapter must exist");
    let config = ContainerLaunchConfig {
        name: "pr2388".into(),
        default: true,
        create: vec![adapter.to_str().unwrap().into()],
        cleanup: vec!["/bin/true".into()],
        exec: vec!["/bin/true".into()],
        kill: vec!["/bin/true".into()],
        inspect: vec!["/bin/true".into()],
        repo_bound: false,
        repository: None,
    };
    let selector = SelectContainerConfig::new(
        Arc::new(CheckoutConfig(config)),
        Arc::new(EmbeddedScriptIntegrity),
    );
    let result = selector.execute(&SelectContainerConfigRequest {
        source: ContainerConfigSource::LaunchingAgent,
        name: Some("pr2388".into()),
    });
    if adapter.ends_with("standard/scripts/create.sh") {
        assert_eq!(
            EmbeddedScriptIntegrity.verify(&adapter),
            StandardScriptVerdict::Differs
        );
        assert!(
            matches!(
                &result,
                Err(crate::application::subagents::dto::SelectContainerConfigError::StandardScriptAltered {
                    script, verdict: StandardScriptVerdict::Differs, ..
                }) if script == &adapter
            ),
            "reserved custom adapter must fail specifically integrity: {result:?}"
        );
    }
    assert!(
        result.is_ok(),
        "production selection refused custom adapter: {result:?}"
    );
    assert_eq!(
        EmbeddedScriptIntegrity.verify(&adapter),
        StandardScriptVerdict::NotStandard
    );
}
