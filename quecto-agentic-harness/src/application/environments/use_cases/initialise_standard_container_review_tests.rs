//! Review-round rules of `InitialiseStandardContainer` (#2024 S4e): any
//! http(s) userinfo is a credential; a re-init keeps the existing entry's
//! `--repo`/`--image`; `--refresh` rewrites differing assets; a project
//! below the checkout's toplevel is refused; the build command is
//! shell-quoted.
use std::path::PathBuf;

use super::rig_tests::*;
use crate::application::environments::dto::{
    ContainerConfigDocument, EntryValueChange, InitialiseStandardContainerError, RepositoryOrigin,
};
use crate::application::environments::ports::ContainerConfigRosterReport;

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
    request.image = Some("my image".into());
    let report = rig.use_case.execute(&request).unwrap();
    assert_eq!(
        report.build_command, "build -t 'my image' /plain/dir/.quecto/containers/standard",
        "the image is quoted the same way"
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
