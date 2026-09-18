use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::ContainerStatus;
use crate::application::environments::dto::{
    AssetOutcome, AssetState, CheckStatus, ContainerAsset, ContainerAssetCatalogue,
    ContainerConfigEntry, ContainerConfigLayer, ContainerRuntimeTarget, DiagnosableContainerConfig,
    PreflightCheck,
};
use crate::application::environments::ports::{
    ContainerAssetStore, ContainerConfigLookup, ContainerConfigRoster, ContainerConfigRosterReport,
    ContainerRuntimePreflight,
};

struct FixedAssets(Vec<AssetState>);

impl ContainerAssetStore for FixedAssets {
    fn catalogue(&self) -> ContainerAssetCatalogue {
        ContainerAssetCatalogue {
            version: 3,
            assets: self
                .0
                .iter()
                .enumerate()
                .map(|(i, _)| ContainerAsset {
                    path: format!("a{i}"),
                    contents: vec![],
                    executable: false,
                })
                .collect(),
        }
    }

    fn observe(&self, _: &Path, asset: &ContainerAsset) -> Result<AssetState, String> {
        let index: usize = asset.path[1..].parse().unwrap();
        Ok(self.0[index])
    }

    fn materialise(&self, _: &Path, _: &ContainerAsset) -> Result<AssetOutcome, String> {
        unreachable!("status never writes")
    }
}

struct FixedRoster(Result<ContainerConfigRosterReport, String>);

impl ContainerConfigRoster for FixedRoster {
    fn roster(&self) -> Result<ContainerConfigRosterReport, String> {
        self.0.clone()
    }

    fn revision(&self) -> String {
        String::new()
    }
}

struct RecordingLookup {
    answer: Result<DiagnosableContainerConfig, String>,
    seen: Mutex<Vec<ContainerRuntimeTarget>>,
}

impl ContainerConfigLookup for RecordingLookup {
    fn lookup(
        &self,
        target: &ContainerRuntimeTarget,
    ) -> Result<DiagnosableContainerConfig, String> {
        self.seen.lock().unwrap().push(target.clone());
        self.answer.clone()
    }
}

struct FixedPreflight(Result<Vec<PreflightCheck>, String>);

impl ContainerRuntimePreflight for FixedPreflight {
    fn preflight(&self, _: &DiagnosableContainerConfig) -> Result<Vec<PreflightCheck>, String> {
        self.0.clone()
    }
}

fn entry(name: &str, layer: ContainerConfigLayer) -> ContainerConfigEntry {
    ContainerConfigEntry {
        name: name.into(),
        default: true,
        layer,
        repository: Some("https://example.test/r".into()),
        problem: None,
        joinable: true,
    }
}

fn check(name: &str, status: CheckStatus) -> PreflightCheck {
    PreflightCheck {
        name: name.into(),
        status,
        detail: format!("{name} detail"),
        remedy: String::new(),
    }
}

fn config() -> DiagnosableContainerConfig {
    DiagnosableContainerConfig {
        name: "standard".into(),
        create: vec!["create".into()],
        diagnostics: vec![],
    }
}

#[test]
fn a_complete_setup_is_healthy_and_the_image_check_comes_from_the_entrys_preflight() {
    let lookup = Arc::new(RecordingLookup {
        answer: Ok(config()),
        seen: Mutex::new(vec![]),
    });
    let status = ContainerStatus::new(
        Arc::new(FixedAssets(vec![
            AssetState::Identical,
            AssetState::Identical,
        ])),
        Arc::new(FixedRoster(Ok(ContainerConfigRosterReport {
            configs: vec![
                entry("other", ContainerConfigLayer::Global),
                entry("standard", ContainerConfigLayer::Overlay),
            ],
            ..Default::default()
        }))),
        lookup.clone(),
        Arc::new(FixedPreflight(Ok(vec![
            check("jq", CheckStatus::Passed),
            check("image", CheckStatus::Passed),
        ]))),
    )
    .execute(Path::new("/p"));
    assert!(status.healthy(), "{status:?}");
    assert_eq!(
        status.assets_dir,
        Path::new("/p/.quecto/containers/standard")
    );
    assert_eq!(status.version, 3);
    assert_eq!(status.assets_present(), 2);
    assert_eq!(
        status.entry.as_ref().unwrap().layer,
        ContainerConfigLayer::Overlay
    );
    assert_eq!(status.image.as_ref().unwrap().detail, "image detail");
    assert_eq!(
        lookup.seen.lock().unwrap()[0],
        ContainerRuntimeTarget {
            name: Some("standard".into())
        }
    );
}

#[test]
fn a_missing_bundle_and_no_entry_run_no_preflight() {
    let status = ContainerStatus::new(
        Arc::new(FixedAssets(vec![AssetState::Missing, AssetState::Differs])),
        Arc::new(FixedRoster(Ok(ContainerConfigRosterReport::default()))),
        Arc::new(RecordingLookup {
            answer: Err("unreachable".into()),
            seen: Mutex::new(vec![]),
        }),
        Arc::new(FixedPreflight(Err("unreachable".into()))),
    )
    .execute(Path::new("/p"));
    assert!(!status.healthy());
    assert_eq!(status.assets_present(), 1);
    assert_eq!(status.assets_differing(), 1);
    assert!(status.entry.is_none());
    assert!(status.image.is_none());
    assert!(status.preflight_error.is_none());
    assert_eq!(
        status.assets[0].0,
        PathBuf::from("/p/.quecto/containers/standard/a0")
    );
}

#[test]
fn a_failed_image_check_a_withheld_overlay_and_a_silent_preflight_are_reported_not_hidden() {
    let build = |roster: Result<ContainerConfigRosterReport, String>,
                 preflight: Result<Vec<PreflightCheck>, String>| {
        ContainerStatus::new(
            Arc::new(FixedAssets(vec![AssetState::Identical])),
            Arc::new(FixedRoster(roster)),
            Arc::new(RecordingLookup {
                answer: Ok(config()),
                seen: Mutex::new(vec![]),
            }),
            Arc::new(FixedPreflight(preflight)),
        )
        .execute(Path::new("/p"))
    };
    let with_entry = ContainerConfigRosterReport {
        configs: vec![entry("standard", ContainerConfigLayer::Overlay)],
        ..Default::default()
    };
    let failed = build(
        Ok(with_entry.clone()),
        Ok(vec![check("image", CheckStatus::Failed)]),
    );
    assert!(!failed.healthy());
    assert_eq!(failed.image.unwrap().status, CheckStatus::Failed);

    let withheld = build(
        Ok(ContainerConfigRosterReport {
            configs: vec![],
            overlay_withheld: true,
            diagnostics: vec!["untrusted".into()],
        }),
        Ok(vec![]),
    );
    assert!(withheld.overlay_withheld && !withheld.healthy());
    assert_eq!(withheld.diagnostics, ["untrusted"]);

    let silent = build(Ok(with_entry.clone()), Err("script refused".into()));
    assert!(!silent.healthy());
    assert_eq!(silent.preflight_error.as_deref(), Some("script refused"));
    assert!(silent.diagnostics.contains(&"script refused".to_string()));

    let unreadable = build(Err("no configuration".into()), Ok(vec![]));
    assert!(unreadable.entry.is_none());
    assert_eq!(unreadable.diagnostics, ["no configuration"]);
}
