//! The golden fixtures are frozen (#2283 review M2): the checked-in
//! `MANIFEST` names every fixture with its SHA-256, may only shrink
//! ([`GOLDEN_CEILING`]), and every fixture is one some scenario loads.
//! A new scenario asserts the Rust board's answers directly: a golden is
//! never re-recorded or edited (the Python board that recorded them is
//! gone), and a fixture no scenario loads is dead weight to delete.
use std::collections::BTreeMap;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::swarm_board_diff_runs::swarm_board_diff::golden::{GOLDEN_DIR, manifest, sha256};

/// The most fixtures the manifest may name: decrease-only.
const GOLDEN_CEILING: usize = 368;

/// Every fixture under the golden folder, relative, with its digest.
fn fixtures() -> BTreeMap<String, String> {
    let mut found = BTreeMap::new();
    let root = Path::new(GOLDEN_DIR);
    for scenario in std::fs::read_dir(root).expect("the golden folder") {
        let scenario = scenario.expect("a golden entry").path();
        if !scenario.is_dir() {
            continue;
        }
        for fixture in std::fs::read_dir(&scenario).expect("a scenario folder") {
            let path = fixture.expect("a fixture").path();
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let bytes = std::fs::read(&path).expect("read a fixture");
            found.insert(relative, sha256(&bytes));
        }
    }
    found
}

#[test]
fn every_golden_fixture_is_in_the_manifest_with_its_digest() {
    let listed = manifest();
    assert!(
        listed.len() <= GOLDEN_CEILING,
        "the golden manifest only shrinks: {} fixtures, ceiling {GOLDEN_CEILING}",
        listed.len()
    );
    assert!(
        listed.len() > 300,
        "the manifest names the frozen scenarios"
    );
    assert_eq!(
        fixtures(),
        listed,
        "the fixtures on disk are the manifest's"
    );
}

/// Re-runs this binary's golden scenarios with every load recorded, and
/// requires the loads to be the manifest's fixtures, all of them.
#[test]
fn every_golden_fixture_is_loaded_by_a_scenario() {
    const LOADED: &str = "QUECTO_SWARM_GOLDEN_LOADED";
    if std::env::var_os(LOADED).is_some() {
        // The re-run's own copy of this test: the parent checks.
        return;
    }
    let scratch = tempfile::tempdir().expect("a scratch folder");
    let log = scratch.path().join("loaded");
    let status = Command::new(std::env::current_exe().expect("this test binary"))
        .arg("swarm_board_")
        .env(LOADED, &log)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .status()
        .expect("re-run the golden scenarios");
    assert!(status.success(), "the golden scenarios pass");
    let loaded: std::collections::BTreeSet<String> = std::fs::read_to_string(&log)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect();
    let listed: std::collections::BTreeSet<String> = manifest().into_keys().collect();
    let unloaded: Vec<&String> = listed.difference(&loaded).collect();
    assert!(
        unloaded.is_empty(),
        "fixtures no scenario loads: {unloaded:?}"
    );
    let unlisted: Vec<&String> = loaded.difference(&listed).collect();
    assert!(
        unlisted.is_empty(),
        "loaded fixtures the manifest lacks: {unlisted:?}"
    );
}
