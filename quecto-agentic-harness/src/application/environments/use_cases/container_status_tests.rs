use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::ContainerStatus;
use crate::application::environments::dto::{
    AssetOutcome, AssetState, CheckStatus, ContainerAsset, ContainerAssetCatalogue,
    ContainerConfigEntry, ContainerConfigLayer, ContainerRuntimeTarget, DiagnosableContainerConfig,
    PreflightCheck, StandardDefault,
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
            build_command: String::new(),
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

    fn observe(&self, _: &Path, _: &Path, asset: &ContainerAsset) -> Result<AssetState, String> {
        let index: usize = asset.path[1..].parse().unwrap();
        match self.0[index] {
            AssetState::Refused => Err(format!("{} is a symbolic link", asset.path)),
            state => Ok(state),
        }
    }

    fn materialise(&self, _: &Path, _: &Path, _: &ContainerAsset) -> Result<AssetOutcome, String> {
        unreachable!("status never writes")
    }

    fn refresh(&self, _: &Path, _: &Path, _: &ContainerAsset) -> Result<AssetOutcome, String> {
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
        inspect: vec![],
        cleanup: vec![],
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
        Arc::new(FixedAssets(vec![
            AssetState::Missing,
            AssetState::Differs,
            AssetState::Refused,
        ])),
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
    assert_eq!(status.assets[2].1, AssetState::Refused);
    assert_eq!(status.diagnostics, ["a2 is a symbolic link"]);
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

    // A present image that cannot host an agent, or lacks a tool it
    // declares, is not ready either: status reports the first image check
    // that failed, not only the lookup (#2073).
    for failing in ["image-base", "required-tools"] {
        let mut checks = vec![
            check("image", CheckStatus::Passed),
            check("image-base", CheckStatus::Passed),
            check("required-tools", CheckStatus::Passed),
            check("repo", CheckStatus::Failed),
        ];
        checks
            .iter_mut()
            .find(|check| check.name == failing)
            .unwrap()
            .status = CheckStatus::Failed;
        let unusable = build(Ok(with_entry.clone()), Ok(checks));
        assert!(!unusable.healthy(), "{failing}");
        assert_eq!(unusable.image.unwrap().name, failing);
    }
    let usable = build(
        Ok(with_entry.clone()),
        Ok(vec![
            check("image", CheckStatus::Passed),
            check("image-base", CheckStatus::Passed),
            check("required-tools", CheckStatus::Passed),
            check("repo", CheckStatus::Failed),
        ]),
    );
    assert!(usable.healthy());
    assert_eq!(usable.image.unwrap().name, "image");

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

// ─── Standard is this repo's default (#2035) ─────────────────────────────────

fn status_for(
    entries: Vec<ContainerConfigEntry>,
) -> crate::application::environments::dto::StandardContainerStatus {
    ContainerStatus::new(
        Arc::new(FixedAssets(vec![AssetState::Identical])),
        Arc::new(FixedRoster(Ok(ContainerConfigRosterReport {
            configs: entries,
            ..Default::default()
        }))),
        Arc::new(RecordingLookup {
            answer: Ok(config()),
            seen: Mutex::new(vec![]),
        }),
        Arc::new(FixedPreflight(Ok(vec![check(
            "image",
            CheckStatus::Passed,
        )]))),
    )
    .execute(Path::new("/p"))
}

#[test]
fn a_labelled_overlay_standard_entry_is_the_repos_default() {
    let status = status_for(vec![
        entry("other", ContainerConfigLayer::Global),
        entry("standard", ContainerConfigLayer::Overlay),
    ]);
    assert_eq!(status.standard_default, Some(StandardDefault::RepoDefault));
    assert!(status.healthy());
}

#[test]
fn an_overlay_standard_entry_whose_label_was_removed_is_still_the_default_by_rule_and_says_so() {
    let mut unlabelled = entry("standard", ContainerConfigLayer::Overlay);
    unlabelled.default = false;
    let status = status_for(vec![
        entry("other", ContainerConfigLayer::Global),
        unlabelled,
    ]);
    assert_eq!(status.standard_default, Some(StandardDefault::LabelRemoved));
    assert!(status.healthy(), "the rule keeps container: true working");
    assert_eq!(
        status.diagnostics,
        [
            "the overlay's standard entry lost its \"default\": true label (removed with `quecto config unset --local`; a raw edit would have un-trusted the overlay); container: true still selects it (a repo's standard container is its default) — restore the label with `quecto container init --refresh` or `quecto config set --local container_configs.standard.default true`"
        ]
    );
}

#[test]
fn a_global_standard_entry_is_not_the_repos_and_its_label_is_reported_as_is() {
    let labelled = status_for(vec![entry("standard", ContainerConfigLayer::Global)]);
    assert_eq!(
        labelled.standard_default,
        Some(StandardDefault::GlobalEntry { labelled: true })
    );
    let mut unlabelled = entry("standard", ContainerConfigLayer::Global);
    unlabelled.default = false;
    let status = status_for(vec![
        entry("other", ContainerConfigLayer::Global),
        unlabelled,
    ]);
    assert_eq!(
        status.standard_default,
        Some(StandardDefault::GlobalEntry { labelled: false })
    );
    assert!(status.diagnostics.is_empty(), "{:?}", status.diagnostics);
}

#[test]
fn no_entry_means_no_default_state() {
    let status = status_for(vec![entry("other", ContainerConfigLayer::Global)]);
    assert_eq!(status.standard_default, None);
}

#[test]
fn a_broken_overlay_standard_entry_is_refused_not_the_repos_default_whatever_its_label() {
    let mut broken = entry("standard", ContainerConfigLayer::Overlay);
    broken.problem = Some("missing create argv".into());
    let status = status_for(vec![entry("other", ContainerConfigLayer::Global), broken]);
    assert_eq!(status.standard_default, Some(StandardDefault::Refused));
    assert!(
        !status
            .diagnostics
            .iter()
            .any(|line| line.contains("still selects it")),
        "{:?}",
        status.diagnostics
    );
}
