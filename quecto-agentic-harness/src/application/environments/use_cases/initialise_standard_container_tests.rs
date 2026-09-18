use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::InitialiseStandardContainer;
use crate::application::environments::dto::{
    AssetOutcome, AssetState, ContainerAsset, ContainerAssetCatalogue, ContainerConfigDocument,
    ContainerConfigEntry, ContainerConfigLayer, EntryValueChange, InitialiseStandardContainerError,
    PersistedContainerConfig, RepositoryOrigin, StandardContainerRequest,
};
use crate::application::environments::ports::{
    ContainerAssetStore, ContainerConfigPersistence, ContainerConfigRoster,
    ContainerConfigRosterReport, WorkspaceOrigin,
};

/// An in-memory bundle: the "disk" is a map of path → bytes.
struct MemoryAssets {
    disk: Mutex<BTreeMap<PathBuf, Vec<u8>>>,
    /// Destinations `observe` refuses (a symbolic link in their place).
    refuse: Mutex<std::collections::BTreeSet<PathBuf>>,
}

fn catalogue() -> ContainerAssetCatalogue {
    ContainerAssetCatalogue {
        version: 7,
        build_command: "build -t {image} {dir}\n".into(),
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

    fn observe(&self, _: &Path, dir: &Path, asset: &ContainerAsset) -> Result<AssetState, String> {
        let path = dir.join(&asset.path);
        if self.refuse.lock().unwrap().contains(&path) {
            return Err(format!("{} is a symbolic link", path.display()));
        }
        Ok(match self.disk.lock().unwrap().get(&path) {
            None => AssetState::Missing,
            Some(bytes) if *bytes == asset.contents => AssetState::Identical,
            Some(_) => AssetState::Differs,
        })
    }

    fn materialise(
        &self,
        root: &Path,
        dir: &Path,
        asset: &ContainerAsset,
    ) -> Result<AssetOutcome, String> {
        assert!(
            dir.starts_with(root),
            "{} not below {}",
            dir.display(),
            root.display()
        );
        let state = self.observe(root, dir, asset)?;
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
            AssetState::Refused => unreachable!(),
        })
    }

    fn refresh(
        &self,
        root: &Path,
        dir: &Path,
        asset: &ContainerAsset,
    ) -> Result<AssetOutcome, String> {
        match self.observe(root, dir, asset)? {
            AssetState::Differs => {
                self.disk
                    .lock()
                    .unwrap()
                    .insert(dir.join(&asset.path), asset.contents.clone());
                Ok(AssetOutcome::Refreshed)
            }
            _ => self.materialise(root, dir, asset),
        }
    }
}

struct FixedOrigin {
    origin: Result<Option<String>, String>,
    /// The toplevel git reports for any checkout asked about; `None` is
    /// "not a checkout".
    toplevel: Mutex<Option<PathBuf>>,
}

impl WorkspaceOrigin for FixedOrigin {
    fn origin(&self, _: &Path) -> Result<Option<String>, String> {
        self.origin.clone()
    }

    fn toplevel(&self, _: &Path) -> Result<Option<PathBuf>, String> {
        Ok(self.toplevel.lock().unwrap().clone())
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
    refuse_existing: Mutex<Option<String>>,
}

impl ContainerConfigPersistence for RecordingPersistence {
    fn check(&self) -> Result<(), String> {
        self.refuse.clone().map_or(Ok(()), Err)
    }

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

    fn existing(&self, name: &str) -> Result<Option<ContainerConfigDocument>, String> {
        if let Some(reason) = self.refuse_existing.lock().unwrap().as_ref() {
            return Err(reason.clone());
        }
        Ok(self
            .written
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(written, _)| written == name)
            .map(|(_, entry)| entry.clone()))
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
    origin: Arc<FixedOrigin>,
    use_case: InitialiseStandardContainer,
}

fn build_rig(
    origin: Result<Option<String>, String>,
    roster: ContainerConfigRosterReport,
    refuse: Option<String>,
) -> Rig {
    let assets = Arc::new(MemoryAssets {
        disk: Mutex::new(BTreeMap::new()),
        refuse: Mutex::new(Default::default()),
    });
    let persistence = Arc::new(RecordingPersistence {
        written: Mutex::new(vec![]),
        refuse,
        refuse_existing: Mutex::new(None),
    });
    let origin = Arc::new(FixedOrigin {
        origin,
        toplevel: Mutex::new(Some(PathBuf::from("/p"))),
    });
    let use_case = InitialiseStandardContainer::new(
        assets.clone(),
        origin.clone(),
        Arc::new(FixedRoster(Ok(roster))),
        persistence.clone(),
        PathBuf::from("/base"),
    );
    Rig {
        assets,
        persistence,
        origin,
        use_case,
    }
}

fn request(project: &str) -> StandardContainerRequest {
    StandardContainerRequest {
        project: PathBuf::from(project),
        repository: None,
        image: None,
        dry_run: false,
        refresh: false,
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
    assert_eq!(
        report.build_command,
        "build -t quecto-box:local /p/.quecto/containers/standard"
    );
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
fn a_second_init_keeps_the_existing_repo_and_image_unless_the_flag_is_given() {
    let rig = build_rig(
        Ok(Some("https://example.test/origin".into())),
        ContainerConfigRosterReport {
            configs: vec![entry("standard", true)],
            ..Default::default()
        },
        None,
    );
    let mut first = request("/p");
    first.repository = Some("https://example.test/explicit".into());
    first.image = Some("mine:1".into());
    let first = rig.use_case.execute(&first).unwrap();
    assert_eq!(first.repository_change, None);
    assert_eq!(first.image_change, None);
    // No flags: the existing entry's values stand, the origin is not
    // re-derived, and the report says what was kept.
    let second = rig.use_case.execute(&request("/p")).unwrap();
    assert_eq!(
        second.repository.as_deref(),
        Some("https://example.test/explicit")
    );
    assert_eq!(second.repository_origin, RepositoryOrigin::ExistingEntry);
    assert_eq!(second.image, "mine:1");
    assert_eq!(second.repository_change, Some(EntryValueChange::Kept));
    assert_eq!(second.image_change, Some(EntryValueChange::Kept));
    assert_eq!(first.entry.entry, second.entry.entry);
    // A flag rewrites that value and the report says so, with the old one.
    let mut third = request("/p");
    third.image = Some("mine:2".into());
    let third = rig.use_case.execute(&third).unwrap();
    assert_eq!(third.image, "mine:2");
    assert_eq!(
        third.repository.as_deref(),
        Some("https://example.test/explicit")
    );
    assert_eq!(third.repository_change, Some(EntryValueChange::Kept));
    assert_eq!(
        third.image_change,
        Some(EntryValueChange::Rewrote {
            previous: Some("mine:1".into())
        })
    );
    assert_eq!(
        argv(&third.entry.entry, "create")[5..],
        ["--image", "mine:2"]
    );
    let mut fourth = request("/p");
    fourth.repository = Some("https://example.test/other".into());
    let fourth = rig.use_case.execute(&fourth).unwrap();
    assert_eq!(fourth.repository_origin, RepositoryOrigin::Explicit);
    assert_eq!(
        fourth.repository_change,
        Some(EntryValueChange::Rewrote {
            previous: Some("https://example.test/explicit".into())
        })
    );
    assert_eq!(fourth.image, "mine:2");
    // The same flag value again is a keep, not a rewrite.
    let mut fifth = request("/p");
    fifth.repository = Some("https://example.test/other".into());
    let fifth = rig.use_case.execute(&fifth).unwrap();
    assert_eq!(fifth.repository_change, Some(EntryValueChange::Kept));
}

#[test]
fn a_kept_existing_repository_is_still_judged_for_credentials() {
    let rig = build_rig(Ok(None), ContainerConfigRosterReport::default(), None);
    rig.persistence.written.lock().unwrap().push((
        "standard".into(),
        ContainerConfigDocument {
            create: vec![
                "/p/.quecto/containers/standard/scripts/create.sh".into(),
                "--repo".into(),
                "https://ghp_TOKEN@example.test/r".into(),
            ],
            ..Default::default()
        },
    ));
    let text = rig
        .use_case
        .execute(&request("/p"))
        .unwrap_err()
        .to_string();
    assert!(text.contains("existing standard entry"), "{text}");
    assert!(text.contains("carries a credential"), "{text}");
    assert!(!text.contains("ghp_TOKEN"), "{text}");
    // An unreadable existing entry is an error, never silently a fresh one.
    let rig = build_rig(Ok(None), ContainerConfigRosterReport::default(), None);
    *rig.persistence.refuse_existing.lock().unwrap() = Some("overlay unreadable".into());
    let text = rig
        .use_case
        .execute(&request("/p"))
        .unwrap_err()
        .to_string();
    assert!(text.contains("overlay unreadable"), "{text}");
}

#[test]
fn a_project_below_the_checkouts_toplevel_is_refused_naming_the_toplevel() {
    let rig = build_rig(Ok(None), ContainerConfigRosterReport::default(), None);
    *rig.origin.toplevel.lock().unwrap() = Some(PathBuf::from("/p"));
    let error = rig.use_case.execute(&request("/p/sub")).unwrap_err();
    assert_eq!(
        error,
        InitialiseStandardContainerError::NotRepositoryRoot {
            project: PathBuf::from("/p/sub"),
            toplevel: PathBuf::from("/p"),
        }
    );
    let text = error.to_string();
    assert!(text.contains("/p/sub is not the repository root"), "{text}");
    assert!(text.contains("--project /p"), "{text}");
    assert!(rig.persistence.written.lock().unwrap().is_empty());
    assert!(rig.assets.disk.lock().unwrap().is_empty());
    // A dry run refuses the same.
    let mut dry = request("/p/sub");
    dry.dry_run = true;
    assert!(rig.use_case.execute(&dry).is_err());
    // The toplevel itself, and a directory that is no checkout at all,
    // are accepted.
    assert!(rig.use_case.execute(&request("/p")).is_ok());
    *rig.origin.toplevel.lock().unwrap() = None;
    assert!(rig.use_case.execute(&request("/p/sub")).is_ok());
}

#[test]
fn the_build_command_quotes_a_bundle_directory_the_shell_would_split_or_expand() {
    let rig = build_rig(Ok(None), ContainerConfigRosterReport::default(), None);
    *rig.origin.toplevel.lock().unwrap() = None;
    let report = rig
        .use_case
        .execute(&request("/my projects/it's here"))
        .unwrap();
    assert_eq!(
        report.build_command,
        "build -t quecto-box:local '/my projects/it'\\''s here/.quecto/containers/standard'"
    );
    let mut request = request("/plain/dir");
    request.image = Some("mine:1".into());
    let report = rig.use_case.execute(&request).unwrap();
    assert_eq!(
        report.build_command, "build -t mine:1 /plain/dir/.quecto/containers/standard",
        "a plain path is left bare"
    );
    let mut request = request;
    request.image = Some("my image".into());
    let report = rig.use_case.execute(&request).unwrap();
    assert_eq!(
        report.build_command, "build -t 'my image' /plain/dir/.quecto/containers/standard",
        "the image is quoted the same way"
    );
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
fn refresh_rewrites_a_differing_asset_with_the_embedded_bytes_and_reports_it() {
    let rig = build_rig(Ok(None), ContainerConfigRosterReport::default(), None);
    let create = PathBuf::from("/p/.quecto/containers/standard/scripts/create.sh");
    rig.assets
        .disk
        .lock()
        .unwrap()
        .insert(create.clone(), b"edited".to_vec());
    // A dry run says what a refresh would do and touches nothing.
    let mut dry = request("/p");
    dry.refresh = true;
    dry.dry_run = true;
    let report = rig.use_case.execute(&dry).unwrap();
    assert_eq!(report.refreshed, std::slice::from_ref(&create));
    assert!(report.differing.is_empty());
    assert_eq!(rig.assets.disk.lock().unwrap()[&create], b"edited".to_vec());
    let mut refresh = request("/p");
    refresh.refresh = true;
    let report = rig.use_case.execute(&refresh).unwrap();
    assert_eq!(report.refreshed, std::slice::from_ref(&create));
    assert!(report.differing.is_empty());
    assert_eq!(
        report.written,
        [PathBuf::from(
            "/p/.quecto/containers/standard/Containerfile"
        )]
    );
    assert_eq!(
        rig.assets.disk.lock().unwrap()[&create],
        b"#!/bin/sh".to_vec()
    );
    // Without --refresh the edit is kept, as before.
    rig.assets
        .disk
        .lock()
        .unwrap()
        .insert(create.clone(), b"edited again".to_vec());
    let report = rig.use_case.execute(&request("/p")).unwrap();
    assert_eq!(report.differing, std::slice::from_ref(&create));
    assert!(report.refreshed.is_empty());
    assert_eq!(
        rig.assets.disk.lock().unwrap()[&create],
        b"edited again".to_vec()
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
fn a_persist_refusal_is_the_configuration_capabilitys_own_words_and_writes_no_asset() {
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
    assert!(
        rig.assets.disk.lock().unwrap().is_empty(),
        "no asset written"
    );
    let mut dry = request("/p");
    dry.dry_run = true;
    let error = rig.use_case.execute(&dry).unwrap_err();
    assert!(
        error.to_string().contains("is not trusted"),
        "a dry run refuses too"
    );
}

#[test]
fn an_origin_with_credentials_is_refused_and_never_written() {
    let rig = build_rig(
        Ok(Some("https://user:tok123@example.test/r.git".into())),
        ContainerConfigRosterReport::default(),
        None,
    );
    let error = rig.use_case.execute(&request("/p")).unwrap_err();
    let text = error.to_string();
    assert!(text.contains("checkout's origin remote"), "{text}");
    assert!(text.contains("carries a credential"), "{text}");
    assert!(text.contains("https://***@example.test/r.git"), "{text}");
    assert!(!text.contains("tok123"), "{text}");
    assert!(rig.persistence.written.lock().unwrap().is_empty());
    assert!(rig.assets.disk.lock().unwrap().is_empty());
    // An explicit --repo with credentials is refused the same way.
    let mut explicit = request("/p");
    explicit.repository = Some("https://user:tok123@example.test/r.git".into());
    let text = rig.use_case.execute(&explicit).unwrap_err().to_string();
    assert!(text.contains("the --repo URL"), "{text}");
    assert!(!text.contains("tok123"), "{text}");
    // A bare user in the ssh form names an account, not a secret.
    let rig = build_rig(
        Ok(Some("ssh://git@example.test/org/repo.git".into())),
        ContainerConfigRosterReport::default(),
        None,
    );
    let report = rig.use_case.execute(&request("/p")).unwrap();
    assert_eq!(
        report.repository.as_deref(),
        Some("ssh://git@example.test/org/repo.git")
    );
}

#[test]
fn a_bare_user_in_an_https_url_is_a_token_and_is_refused_from_either_source() {
    // GitHub's documented form carries the token as the whole userinfo,
    // no colon: `https://ghp_xxx@github.com/org/repo`.
    for url in [
        "https://ghp_SECRET@github.com/org/repo.git",
        "http://ghp_SECRET@example.test/r",
        "HTTPS://ghp_SECRET@example.test/r",
    ] {
        let rig = build_rig(
            Ok(Some(url.into())),
            ContainerConfigRosterReport::default(),
            None,
        );
        let text = rig
            .use_case
            .execute(&request("/p"))
            .unwrap_err()
            .to_string();
        assert!(text.contains("checkout's origin remote"), "{url}: {text}");
        assert!(text.contains("carries a credential"), "{url}: {text}");
        assert!(!text.contains("ghp_SECRET"), "{url}: {text}");
        assert!(rig.persistence.written.lock().unwrap().is_empty(), "{url}");
        assert!(rig.assets.disk.lock().unwrap().is_empty(), "{url}");
        let mut explicit = request("/p");
        explicit.repository = Some(url.into());
        let text = rig.use_case.execute(&explicit).unwrap_err().to_string();
        assert!(text.contains("the --repo URL"), "{url}: {text}");
        assert!(!text.contains("ghp_SECRET"), "{url}: {text}");
        assert!(rig.persistence.written.lock().unwrap().is_empty(), "{url}");
    }
    // A bare user in the ssh, git and scp forms names an account.
    for url in [
        "ssh://git@example.test/org/repo.git",
        "git://git@example.test/org/repo.git",
        "git@github.com:org/repo.git",
    ] {
        let rig = build_rig(
            Ok(Some(url.into())),
            ContainerConfigRosterReport::default(),
            None,
        );
        let report = rig.use_case.execute(&request("/p")).unwrap();
        assert_eq!(report.repository.as_deref(), Some(url));
    }
    // A password in those forms is still a secret.
    let rig = build_rig(
        Ok(Some("ssh://git:pw@example.test/org/repo.git".into())),
        ContainerConfigRosterReport::default(),
        None,
    );
    let text = rig
        .use_case
        .execute(&request("/p"))
        .unwrap_err()
        .to_string();
    assert!(text.contains("carries a credential"), "{text}");
    assert!(!text.contains("pw@"), "{text}");
    let rig = build_rig(
        Ok(Some("git:pw@example.test:org/repo.git".into())),
        ContainerConfigRosterReport::default(),
        None,
    );
    let text = rig
        .use_case
        .execute(&request("/p"))
        .unwrap_err()
        .to_string();
    assert!(text.contains("***@example.test:org/repo.git"), "{text}");
    assert!(!text.contains("pw@"), "{text}");
}

#[test]
fn a_refused_destination_is_found_before_the_entry_is_written() {
    let rig = build_rig(Ok(None), ContainerConfigRosterReport::default(), None);
    rig.assets.refuse.lock().unwrap().insert(PathBuf::from(
        "/p/.quecto/containers/standard/scripts/create.sh",
    ));
    for dry_run in [true, false] {
        let mut req = request("/p");
        req.dry_run = dry_run;
        let error = rig.use_case.execute(&req).unwrap_err();
        let text = error.to_string();
        assert!(text.contains("is a symbolic link"), "{text}");
        assert!(!text.contains("already written"), "{text}");
        assert!(rig.persistence.written.lock().unwrap().is_empty());
        assert!(rig.assets.disk.lock().unwrap().is_empty());
    }
}

#[test]
fn an_asset_failure_after_the_entry_landed_names_the_way_out() {
    let error = InitialiseStandardContainerError::Asset {
        path: PathBuf::from("/p/x"),
        reason: "disk full".into(),
        entry_written: true,
    }
    .to_string();
    assert!(error.contains("already written"), "{error}");
    assert!(
        error.contains("quecto config unset --local container_configs.standard"),
        "{error}"
    );
}

#[test]
fn a_relative_base_dir_is_refused() {
    let assets = Arc::new(MemoryAssets {
        disk: Mutex::new(BTreeMap::new()),
        refuse: Mutex::new(Default::default()),
    });
    let use_case = InitialiseStandardContainer::new(
        assets,
        Arc::new(FixedOrigin {
            origin: Ok(None),
            toplevel: Mutex::new(None),
        }),
        Arc::new(FixedRoster(Ok(ContainerConfigRosterReport::default()))),
        Arc::new(RecordingPersistence::default()),
        PathBuf::from("../base"),
    );
    let error = use_case.execute(&request("/p")).unwrap_err();
    assert!(matches!(
        error,
        InitialiseStandardContainerError::BaseDirNotAbsolute(_)
    ));
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
