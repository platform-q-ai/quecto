use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::InitialiseStandardContainer;
use super::rig_tests::*;
use crate::application::environments::dto::{
    ContainerConfigEntry, ContainerConfigLayer, InitialiseStandardContainerError, RepositoryOrigin,
};
use crate::application::environments::ports::ContainerConfigRosterReport;

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
    assert_eq!(report.image, "quecto-p:local");
    assert_eq!(
        report.build_command,
        "build -t quecto-p:local /p/.quecto/containers/standard"
    );
    assert_eq!(report.entry.displaced_default, None);
    assert_eq!(report.entry.overridden_global_default, None);
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
            "quecto-p:local"
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
    assert_eq!(
        second.entry.displaced_default, None,
        "our own label is not a displacement"
    );
    assert!(
        second.entry.entry.default,
        "our own default stays the default"
    );
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
fn a_global_default_is_overridden_for_this_repo_and_named_but_never_displaced() {
    let rig = build_rig(
        Ok(Some("u".into())),
        ContainerConfigRosterReport {
            configs: vec![entry("other", true), entry("more", false)],
            ..Default::default()
        },
        None,
    );
    let report = rig.use_case.execute(&request("/p")).unwrap();
    assert!(
        report.entry.entry.default,
        "standard is always written as the default"
    );
    assert_eq!(report.entry.displaced_default, None);
    assert_eq!(
        report.entry.overridden_global_default.as_deref(),
        Some("other")
    );
    let written = rig.persistence.written.lock().unwrap();
    assert!(written[0].1.default);
    assert_eq!(
        rig.persistence.displaced.lock().unwrap().as_slice(),
        &[None],
        "a global label is overridden by the merge, never displaced"
    );
}

#[test]
fn an_overlay_entry_carrying_the_default_label_is_displaced_in_the_same_write_and_named() {
    let rig = build_rig(
        Ok(Some("u".into())),
        ContainerConfigRosterReport {
            configs: vec![
                entry("global", false),
                ContainerConfigEntry {
                    layer: ContainerConfigLayer::Overlay,
                    ..entry("r", true)
                },
            ],
            ..Default::default()
        },
        None,
    );
    let report = rig.use_case.execute(&request("/p")).unwrap();
    assert!(report.entry.entry.default);
    assert_eq!(report.entry.displaced_default.as_deref(), Some("r"));
    assert_eq!(report.entry.overridden_global_default, None);
    assert_eq!(
        rig.persistence.displaced.lock().unwrap().as_slice(),
        &[Some("r".to_string())]
    );
}

#[test]
fn a_dry_run_names_what_it_would_displace_without_writing() {
    let rig = build_rig(
        Ok(Some("u".into())),
        ContainerConfigRosterReport {
            configs: vec![ContainerConfigEntry {
                layer: ContainerConfigLayer::Overlay,
                ..entry("r", true)
            }],
            ..Default::default()
        },
        None,
    );
    let mut dry = request("/p");
    dry.dry_run = true;
    let report = rig.use_case.execute(&dry).unwrap();
    assert_eq!(report.entry.displaced_default.as_deref(), Some("r"));
    assert!(rig.persistence.written.lock().unwrap().is_empty());
}

#[test]
fn our_own_overlay_label_is_neither_displaced_nor_an_override() {
    let rig = build_rig(
        Ok(Some("u".into())),
        ContainerConfigRosterReport {
            configs: vec![ContainerConfigEntry {
                layer: ContainerConfigLayer::Overlay,
                ..entry("standard", true)
            }],
            ..Default::default()
        },
        None,
    );
    let report = rig.use_case.execute(&request("/p")).unwrap();
    assert_eq!(report.entry.displaced_default, None);
    assert_eq!(report.entry.overridden_global_default, None);
    assert_eq!(
        rig.persistence.displaced.lock().unwrap().as_slice(),
        &[None]
    );
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
    assert_eq!(&create[create.len() - 2..], ["--image", "quecto-p:local"]);
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
