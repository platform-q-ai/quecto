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
