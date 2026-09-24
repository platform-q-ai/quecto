use std::path::Path;
use std::sync::Mutex;

use super::{
    MAX_RESIDUE_INSPECTS, NO_ENVIRONMENT_DIR, Residue, ResidueProbe, STATE_GONE_CONTAINER_RUNNING,
};
use crate::application::environments::dto::{EnvironmentLiveness, StateOnDisk};
use crate::application::environments::ports::EnvironmentProcess;
use crate::application::environments::use_cases::restore_registry_tests::record;
use crate::domain::environment_registry::{EnvironmentRecord, EnvironmentStatus};

/// Every state directory is absent unless listed in `present`; every
/// container answers `live` (by ref, else `Gone`); inspects are counted.
struct Host {
    present: Vec<&'static str>,
    live: Vec<(&'static str, EnvironmentLiveness)>,
    inspected: Mutex<Vec<String>>,
}

impl EnvironmentProcess for Host {
    fn observe(&self, record: &EnvironmentRecord) -> EnvironmentLiveness {
        self.inspected
            .lock()
            .unwrap()
            .push(record.environment_ref.clone());
        self.live
            .iter()
            .find(|(r, _)| *r == record.environment_ref)
            .map(|(_, l)| l.clone())
            .unwrap_or(EnvironmentLiveness::Gone)
    }
    fn cleanup(&self, _: &EnvironmentRecord) -> Result<(), String> {
        Ok(())
    }
    fn state_on_disk(&self, dir: &Path) -> StateOnDisk {
        match self.present.iter().any(|p| Path::new(p) == dir) {
            true => StateOnDisk::Present,
            false => StateOnDisk::Absent,
        }
    }
}

fn host(present: Vec<&'static str>, live: Vec<(&'static str, EnvironmentLiveness)>) -> Host {
    Host {
        present,
        live,
        inspected: Mutex::default(),
    }
}

fn stopped(reference: &str) -> EnvironmentRecord {
    record(reference, EnvironmentStatus::Stopped)
}

#[test]
fn a_box_is_judged_disk_first_then_by_its_own_inspect() {
    let host = host(
        vec!["/state/env-C1"],
        vec![("C3", EnvironmentLiveness::Running)],
    );
    let mut probe = ResidueProbe::new(&host);
    assert_eq!(probe.residue(&stopped("C1")), Residue::StateOnDisk);
    assert_eq!(probe.residue(&stopped("C2")), Residue::Nothing);
    assert_eq!(
        probe.residue(&stopped("C3")),
        Residue::Kept(STATE_GONE_CONTAINER_RUNNING.to_string())
    );
    let mut elsewhere = stopped("C4");
    elsewhere.workspace_path = "/w".into();
    assert_eq!(
        probe.residue(&elsewhere),
        Residue::Kept(NO_ENVIRONMENT_DIR.to_string())
    );
    assert_eq!(
        *host.inspected.lock().unwrap(),
        ["C2", "C3"],
        "C1 left its state: never inspected"
    );
}

#[test]
fn a_probe_stops_inspecting_once_the_runtime_does_not_answer() {
    let host = host(
        vec![],
        vec![("C1", EnvironmentLiveness::Unknown("timed out".into()))],
    );
    let mut probe = ResidueProbe::new(&host);
    assert_eq!(
        probe.residue(&stopped("C1")),
        Residue::Kept("timed out".into())
    );
    assert_eq!(probe.residue(&stopped("C2")), Residue::Deferred);
    assert_eq!(*host.inspected.lock().unwrap(), ["C1"]);
}

#[test]
fn a_probe_inspects_at_most_its_budget() {
    let host = host(vec![], vec![]);
    let mut probe = ResidueProbe::new(&host);
    let judged: Vec<Residue> = (1..=MAX_RESIDUE_INSPECTS + 1)
        .map(|n| probe.residue(&stopped(&format!("C{n}"))))
        .collect();
    assert!(
        judged[..MAX_RESIDUE_INSPECTS]
            .iter()
            .all(|r| *r == Residue::Nothing)
    );
    assert_eq!(judged[MAX_RESIDUE_INSPECTS], Residue::Deferred);
    assert_eq!(host.inspected.lock().unwrap().len(), MAX_RESIDUE_INSPECTS);
    // The disk is still asked once the budget is spent.
    let host = Host {
        present: vec!["/state/env-C99"],
        ..host
    };
    let mut spent = ResidueProbe::new(&host);
    for n in 1..=MAX_RESIDUE_INSPECTS {
        spent.residue(&stopped(&format!("C{n}")));
    }
    assert_eq!(spent.residue(&stopped("C99")), Residue::StateOnDisk);
}
