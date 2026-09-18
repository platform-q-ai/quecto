use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::InitialiseStandardContainer;
use crate::application::environments::dto::{
    AssetOutcome, AssetState, ContainerAsset, ContainerAssetCatalogue, ContainerConfigDocument,
    ContainerConfigEntry, ContainerConfigLayer, InitialiseStandardContainerError,
    PersistedContainerConfig, RepositoryOrigin, StandardContainerRequest,
};
use crate::application::environments::ports::{
    ContainerAssetStore, ContainerConfigPersistence, ContainerConfigRoster,
    ContainerConfigRosterReport, WorkspaceOrigin,
};

/// An in-memory bundle: the "disk" is a map of path → bytes.
struct MemoryAssets {
    disk: Mutex<BTreeMap<PathBuf, Vec<u8>>>,
}

fn catalogue() -> ContainerAssetCatalogue {
    ContainerAssetCatalogue {
        version: 7,
        assets: vec![
            ContainerAsset {
                path: "Containerfile".into(),
                contents: b"FROM x".to_vec(),
                executable: false,
            },
            ContainerAsset {
                path: "scripts/create.sh".into(),
                contents: b"#!/bin/sh".to_vec(),
                executable: true,
            },
        ],
    }
}

impl ContainerAssetStore for MemoryAssets {
    fn catalogue(&self) -> ContainerAssetCatalogue {
        catalogue()
    }

    fn observe(&self, dir: &Path, asset: &ContainerAsset) -> Result<AssetState, String> {
        Ok(
            match self.disk.lock().unwrap().get(&dir.join(&asset.path)) {
                None => AssetState::Missing,
                Some(bytes) if *bytes == asset.contents => AssetState::Identical,
                Some(_) => AssetState::Differs,
            },
        )
    }

    fn materialise(&self, dir: &Path, asset: &ContainerAsset) -> Result<AssetOutcome, String> {
        let state = self.observe(dir, asset)?;
        Ok(match state {
            AssetState::Missing => {
                self.disk
                    .lock()
                    .unwrap()
                    .insert(dir.join(&asset.path), asset.contents.clone());
                AssetOutcome::Written
            }
            AssetState::Identical => AssetOutcome::KeptIdentical,
            AssetState::Differs => AssetOutcome::KeptDiffering,
        })
    }
}

struct FixedOrigin(Result<Option<String>, String>);

impl WorkspaceOrigin for FixedOrigin {
    fn origin(&self, _: &Path) -> Result<Option<String>, String> {
        self.0.clone()
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

#[derive(Default)]
struct RecordingPersistence {
    written: Mutex<Vec<(String, ContainerConfigDocument)>>,
    refuse: Option<String>,
}

impl ContainerConfigPersistence for RecordingPersistence {
    fn persist(
        &self,
        name: &str,
        entry: &ContainerConfigDocument,
    ) -> Result<PersistedContainerConfig, String> {
        if let Some(reason) = &self.refuse {
            return Err(reason.clone());
        }
        self.written
            .lock()
            .unwrap()
            .push((name.to_string(), entry.clone()));
        Ok(PersistedContainerConfig {
            path: PathBuf::from("/p/.quecto/config.json"),
            created: true,
        })
    }

    fn location(&self) -> Option<PathBuf> {
        Some(PathBuf::from("/p/.quecto/config.json"))
    }
}

fn entry(name: &str, default: bool) -> ContainerConfigEntry {
    ContainerConfigEntry {
        name: name.into(),
        default,
        layer: ContainerConfigLayer::Global,
        repository: None,
        problem: None,
        joinable: true,
    }
}

struct Rig {
    assets: Arc<MemoryAssets>,
    persistence: Arc<RecordingPersistence>,
    use_case: InitialiseStandardContainer,
}

fn build_rig(
    origin: Result<Option<String>, String>,
    roster: ContainerConfigRosterReport,
    refuse: Option<String>,
) -> Rig {
    let assets = Arc::new(MemoryAssets {
        disk: Mutex::new(BTreeMap::new()),
    });
    let persistence = Arc::new(RecordingPersistence {
        written: Mutex::new(vec![]),
        refuse,
    });
    let use_case = InitialiseStandardContainer::new(
        assets.clone(),
        Arc::new(FixedOrigin(origin)),
        Arc::new(FixedRoster(Ok(roster))),
        persistence.clone(),
        PathBuf::from("/base"),
    );
    Rig {
        assets,
        persistence,
        use_case,
    }
}

fn request(project: &str) -> StandardContainerRequest {
    StandardContainerRequest {
        project: PathBuf::from(project),
        repository: None,
        image: None,
        dry_run: false,
    }
}

fn argv(entry: &ContainerConfigDocument, key: &str) -> Vec<String> {
    entry
        .argvs()
        .into_iter()
        .find(|(k, _)| *k == key)
        .map(|(_, argv)| argv.to_vec())
        .unwrap()
}

#[test]
fn writes_the_bundle_and_a_default_entry_naming_the_materialised_scripts() {
    let rig = build_rig(
        Ok(Some("https://example.test/origin".into())),
        ContainerConfigRosterReport::default(),
        None,
    );
    let report = rig.use_case.execute(&request("/p")).unwrap();
    assert_eq!(
        report.assets_dir,
        Path::new("/p/.quecto/containers/standard")
    );
    assert_eq!(report.version, 7);
    assert_eq!(
        report.written,
        [
            PathBuf::from("/p/.quecto/containers/standard/Containerfile"),
            PathBuf::from("/p/.quecto/containers/standard/scripts/create.sh")
        ]
    );
    assert!(report.kept.is_empty() && report.differing.is_empty());
    assert_eq!(
        report.repository.as_deref(),
        Some("https://example.test/origin")
    );
    assert_eq!(report.repository_origin, RepositoryOrigin::CheckoutOrigin);
    assert_eq!(report.image, "quecto-box:local");
    assert!(report.entry.default);
    assert_eq!(report.entry.existing_default, None);
    assert_eq!(
        report.entry.path.as_deref(),
        Some(Path::new("/p/.quecto/config.json"))
    );
    let written = rig.persistence.written.lock().unwrap();
    let (name, entry) = &written[0];
    assert_eq!(name, "standard");
    assert!(entry.default);
    assert_eq!(
        argv(entry, "create"),
        [
            "/p/.quecto/containers/standard/scripts/create.sh",
            "--state-dir",
            "/base/container-environments",
            "--repo",
            "https://example.test/origin",
            "--image",
            "quecto-box:local"
        ]
    );
    assert_eq!(
        argv(entry, "exec"),
        [
            "/p/.quecto/containers/standard/scripts/exec.sh",
            "--state-dir",
            "/base/container-environments"
        ]
    );
    assert_eq!(
        argv(entry, "inspect")[0],
        "/p/.quecto/containers/standard/scripts/inspect.sh"
    );
    assert_eq!(
        argv(entry, "kill"),
        [
            "/p/.quecto/containers/standard/scripts/kill.sh",
            "--state-dir",
            "/base/container-environments",
            "--op",
            "kill"
        ]
    );
    assert_eq!(argv(entry, "cleanup").last().unwrap(), "cleanup");
    for key in ["create", "exec", "inspect", "kill", "cleanup"] {
        assert_ne!(argv(entry, key).last().unwrap(), "--", "{key} ends with --");
    }
    assert_eq!(*entry, report.entry.entry);
}

#[test]
fn a_second_init_keeps_every_asset_and_rewrites_the_same_entry() {
    let rig = build_rig(
        Ok(Some("https://example.test/origin".into())),
        ContainerConfigRosterReport {
            configs: vec![entry("standard", true)],
            ..Default::default()
        },
        None,
    );
    let first = rig.use_case.execute(&request("/p")).unwrap();
    let second = rig.use_case.execute(&request("/p")).unwrap();
    assert!(second.written.is_empty());
    assert_eq!(second.kept.len(), 2);
    assert!(second.entry.default, "our own default stays the default");
    let written = rig.persistence.written.lock().unwrap();
    assert_eq!(written[0].1, written[1].1);
    assert_eq!(first.entry.entry, second.entry.entry);
}

#[test]
fn an_edited_asset_is_kept_and_reported_as_differing() {
    let rig = build_rig(Ok(None), ContainerConfigRosterReport::default(), None);
    rig.assets.disk.lock().unwrap().insert(
        PathBuf::from("/p/.quecto/containers/standard/scripts/create.sh"),
        b"edited".to_vec(),
    );
    let report = rig.use_case.execute(&request("/p")).unwrap();
    assert_eq!(
        report.differing,
        [PathBuf::from(
            "/p/.quecto/containers/standard/scripts/create.sh"
        )]
    );
    assert_eq!(
        rig.assets.disk.lock().unwrap()
            [Path::new("/p/.quecto/containers/standard/scripts/create.sh")],
        b"edited".to_vec()
    );
}

#[test]
fn an_existing_default_keeps_its_label_and_ours_is_added_without_it() {
    let rig = build_rig(
        Ok(Some("u".into())),
        ContainerConfigRosterReport {
            configs: vec![entry("other", true), entry("more", false)],
            ..Default::default()
        },
        None,
    );
    let report = rig.use_case.execute(&request("/p")).unwrap();
    assert!(!report.entry.default);
    assert_eq!(report.entry.existing_default.as_deref(), Some("other"));
    let written = rig.persistence.written.lock().unwrap();
    assert!(!written[0].1.default);
}

#[test]
fn an_explicit_repo_wins_and_no_origin_means_a_sandbox() {
    let rig = build_rig(
        Ok(Some("https://example.test/origin".into())),
        ContainerConfigRosterReport::default(),
        None,
    );
    let mut explicit = request("/p");
    explicit.repository = Some("https://example.test/explicit".into());
    let report = rig.use_case.execute(&explicit).unwrap();
    assert_eq!(report.repository_origin, RepositoryOrigin::Explicit);
    assert!(
        argv(&report.entry.entry, "create").contains(&"https://example.test/explicit".to_string())
    );

    let rig = build_rig(Ok(None), ContainerConfigRosterReport::default(), None);
    let report = rig.use_case.execute(&request("/p")).unwrap();
    assert_eq!(report.repository_origin, RepositoryOrigin::Sandbox);
    assert_eq!(report.repository, None);
    let create = argv(&report.entry.entry, "create");
    assert!(!create.contains(&"--repo".to_string()), "{create:?}");
    assert_eq!(&create[create.len() - 2..], ["--image", "quecto-box:local"]);
}

#[test]
fn an_explicit_image_is_baked_into_the_create_argv() {
    let rig = build_rig(Ok(None), ContainerConfigRosterReport::default(), None);
    let mut with_image = request("/p");
    with_image.image = Some("localhost/quecto-standard:test".into());
    let report = rig.use_case.execute(&with_image).unwrap();
    assert_eq!(report.image, "localhost/quecto-standard:test");
    let create = argv(&report.entry.entry, "create");
    assert_eq!(
        &create[create.len() - 2..],
        ["--image", "localhost/quecto-standard:test"]
    );
}

#[test]
fn a_dry_run_writes_nothing_and_reports_what_it_would() {
    let rig = build_rig(
        Ok(Some("u".into())),
        ContainerConfigRosterReport::default(),
        None,
    );
    let mut dry = request("/p");
    dry.dry_run = true;
    let report = rig.use_case.execute(&dry).unwrap();
    assert!(report.dry_run);
    assert_eq!(report.written.len(), 2, "what would be written");
    assert!(rig.assets.disk.lock().unwrap().is_empty());
    assert!(rig.persistence.written.lock().unwrap().is_empty());
    assert_eq!(
        report.entry.path.as_deref(),
        Some(Path::new("/p/.quecto/config.json"))
    );
}

#[test]
fn a_withheld_overlay_refuses_before_any_asset_is_written() {
    let rig = build_rig(
        Ok(None),
        ContainerConfigRosterReport {
            configs: vec![],
            overlay_withheld: true,
            diagnostics: vec!["overlay /p/.quecto/config.json is not trusted".into()],
        },
        None,
    );
    let error = rig.use_case.execute(&request("/p")).unwrap_err();
    let InitialiseStandardContainerError::Configuration(reason) = &error else {
        panic!("{error:?}");
    };
    assert!(reason.contains("quecto config trust"), "{reason}");
    assert!(reason.contains("is not trusted"), "{reason}");
    assert!(rig.assets.disk.lock().unwrap().is_empty());
}

#[test]
fn a_persist_refusal_is_the_configuration_capabilitys_own_words() {
    let rig = build_rig(
        Ok(None),
        ContainerConfigRosterReport::default(),
        Some("overlay /p/.quecto/config.json is not trusted (sha256 abc)".into()),
    );
    let error = rig.use_case.execute(&request("/p")).unwrap_err();
    assert_eq!(
        error.to_string(),
        "overlay /p/.quecto/config.json is not trusted (sha256 abc)"
    );
}

#[test]
fn a_relative_project_and_an_unreadable_origin_are_errors() {
    let rig = build_rig(Ok(None), ContainerConfigRosterReport::default(), None);
    let error = rig.use_case.execute(&request("relative")).unwrap_err();
    assert!(matches!(
        error,
        InitialiseStandardContainerError::ProjectNotAbsolute(_)
    ));
    let rig = build_rig(
        Err("git is not on PATH".into()),
        ContainerConfigRosterReport::default(),
        None,
    );
    let error = rig.use_case.execute(&request("/p")).unwrap_err();
    assert_eq!(
        error.to_string(),
        "cannot read the checkout's origin: git is not on PATH"
    );
}
