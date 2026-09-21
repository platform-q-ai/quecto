//! The Containerfile is the project's own (#2073): init writes a starter
//! only when none exists, never counts the project's version as drift and
//! never replaces it — `--refresh` included. The scripts stay the bundle's.
use std::path::{Path, PathBuf};

use super::rig_tests::*;
use crate::application::environments::ports::ContainerConfigRosterReport;

const CONTAINERFILE: &str = "/p/.quecto/containers/standard/Containerfile";
const CREATE: &str = "/p/.quecto/containers/standard/scripts/create.sh";

fn rig_with_edits() -> Rig {
    let rig = build_rig(Ok(None), ContainerConfigRosterReport::default(), None);
    {
        let mut disk = rig.assets.disk.lock().unwrap();
        disk.insert(PathBuf::from(CONTAINERFILE), b"FROM python".to_vec());
        disk.insert(PathBuf::from(CREATE), b"edited".to_vec());
    }
    rig
}

#[test]
fn the_projects_containerfile_is_kept_as_its_own_and_is_not_drift() {
    let rig = rig_with_edits();
    let report = rig.use_case.execute(&request("/p")).unwrap();
    assert_eq!(report.own, [PathBuf::from(CONTAINERFILE)]);
    assert_eq!(report.differing, [PathBuf::from(CREATE)]);
    assert_eq!(
        rig.assets.disk.lock().unwrap()[Path::new(CONTAINERFILE)],
        b"FROM python".to_vec()
    );
}

#[test]
fn a_refresh_restores_the_scripts_and_never_touches_the_projects_containerfile() {
    for dry_run in [true, false] {
        let rig = rig_with_edits();
        let mut refresh = request("/p");
        refresh.refresh = true;
        refresh.dry_run = dry_run;
        let report = rig.use_case.execute(&refresh).unwrap();
        assert_eq!(report.own, [PathBuf::from(CONTAINERFILE)], "{dry_run}");
        assert_eq!(report.refreshed, [PathBuf::from(CREATE)], "{dry_run}");
        assert!(report.differing.is_empty(), "{dry_run}");
        let disk = rig.assets.disk.lock().unwrap();
        assert_eq!(disk[Path::new(CONTAINERFILE)], b"FROM python".to_vec());
        let expected: &[u8] = if dry_run { b"edited" } else { b"#!/bin/sh" };
        assert_eq!(disk[Path::new(CREATE)], expected, "{dry_run}");
    }
}

#[test]
fn a_symbolic_link_in_the_containerfiles_place_is_still_refused() {
    // Ownership changes what other BYTES mean, not what a link means.
    for refresh in [false, true] {
        let rig = build_rig(Ok(None), ContainerConfigRosterReport::default(), None);
        rig.assets
            .refuse
            .lock()
            .unwrap()
            .insert(PathBuf::from(CONTAINERFILE));
        let mut req = request("/p");
        req.refresh = refresh;
        let text = rig.use_case.execute(&req).unwrap_err().to_string();
        assert!(text.contains("is a symbolic link"), "{text}");
        assert!(rig.persistence.written.lock().unwrap().is_empty());
        assert!(rig.assets.disk.lock().unwrap().is_empty());
    }
}

#[test]
fn a_missing_containerfile_is_written_as_the_starter_even_on_a_refresh() {
    let rig = build_rig(Ok(None), ContainerConfigRosterReport::default(), None);
    let mut refresh = request("/p");
    refresh.refresh = true;
    let report = rig.use_case.execute(&refresh).unwrap();
    assert!(report.written.contains(&PathBuf::from(CONTAINERFILE)));
    assert!(report.own.is_empty());
    assert_eq!(
        rig.assets.disk.lock().unwrap()[Path::new(CONTAINERFILE)],
        b"FROM x".to_vec()
    );
}

// ─── A default image tag per repository (#2073) ─────────────────────────────

/// A rig whose checkout toplevel is `project` (init refuses a project below
/// the toplevel git reports).
fn rig_at(project: &str) -> Rig {
    let rig = build_rig(Ok(None), ContainerConfigRosterReport::default(), None);
    *rig.origin.toplevel.lock().unwrap() = Some(PathBuf::from(project));
    rig
}

#[test]
fn two_projects_get_two_default_image_tags_named_after_their_directories() {
    let tag = |project: &str| {
        let rig = rig_at(project);
        let report = rig.use_case.execute(&request(project)).unwrap();
        let create = argv(&report.entry.entry, "create");
        assert_eq!(create[create.len() - 2], "--image");
        assert_eq!(create[create.len() - 1], report.image);
        report.image
    };
    assert_eq!(tag("/work/shop-api"), "quecto-shop-api:local");
    assert_eq!(tag("/work/quecto"), "quecto-quecto:local");
    assert_ne!(tag("/work/a"), tag("/work/b"));
}

#[test]
fn a_directory_name_becomes_a_valid_image_name() {
    use crate::application::environments::dto::standard_image_for;
    for (project, image) in [
        ("/w/My Project", "quecto-my-project:local"),
        ("/w/API_v2.1", "quecto-api_v2.1:local"),
        ("/w/--weird--", "quecto-weird:local"),
        ("/w/a__b..c", "quecto-a-b-c:local"),
        ("/w/caf\u{e9}", "quecto-caf:local"),
        ("/w/\u{65e5}\u{672c}", "quecto-dev:local"),
        ("/", "quecto-dev:local"),
    ] {
        assert_eq!(standard_image_for(Path::new(project)), image, "{project}");
    }
    let long = format!("/w/{}", "x".repeat(300));
    let image = standard_image_for(Path::new(&long));
    assert_eq!(image, format!("quecto-{}:local", "x".repeat(100)));
}

#[test]
fn an_existing_entry_keeps_the_tag_it_has() {
    use crate::application::environments::dto::STANDARD_CONTAINER_IMAGE;
    let rig = rig_at("/work/shop-api");
    let mut first = request("/work/shop-api");
    first.image = Some(STANDARD_CONTAINER_IMAGE.into());
    rig.use_case.execute(&first).unwrap();
    let again = rig.use_case.execute(&request("/work/shop-api")).unwrap();
    assert_eq!(again.image, STANDARD_CONTAINER_IMAGE);
}
