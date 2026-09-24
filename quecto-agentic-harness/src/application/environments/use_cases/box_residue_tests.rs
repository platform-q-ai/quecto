use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::{NO_ENVIRONMENT_DIR, Residue, ResidueProbe, STATE_GONE_CONTAINER_RUNNING};
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
fn a_record_the_runtime_cannot_answer_for_holds_back_no_other() {
    // Records are judged in ref order on every restore: a first record
    // whose inspect fails (no argv, an unrecognised status) must not keep
    // every later one for ever.
    let host = host(
        vec![],
        vec![("C1", EnvironmentLiveness::Unknown("no inspect argv".into()))],
    );
    let mut probe = ResidueProbe::new(&host);
    assert_eq!(
        probe.residue(&stopped("C1")),
        Residue::Kept("no inspect argv".into())
    );
    assert_eq!(probe.residue(&stopped("C2")), Residue::Nothing);
    assert_eq!(probe.residue(&stopped("C3")), Residue::Nothing);
    assert_eq!(*host.inspected.lock().unwrap(), ["C1", "C2", "C3"]);
}

#[test]
fn a_probe_inspects_until_its_time_budget_is_spent() {
    // Each inspect takes 4 s on a fake clock; the budget is 10 s: inspects
    // start at 0, 4 and 8 s, the fourth would start at 12 s and waits.
    let host = host(vec!["/state/env-C9"], vec![]);
    let now = Arc::new(Mutex::new(Duration::ZERO));
    let clock = {
        let now = now.clone();
        let inspected = Arc::new(Mutex::new(0usize));
        let host_inspected = &host.inspected;
        Box::new(move || {
            let count = host_inspected.lock().unwrap().len();
            let mut seen = inspected.lock().unwrap();
            let mut t = now.lock().unwrap();
            *t += Duration::from_secs(4) * (count - *seen) as u32;
            *seen = count;
            *t
        })
    };
    let mut probe = ResidueProbe::with_clock(&host, clock, Duration::from_secs(10));
    let judged: Vec<Residue> = (1..=4)
        .map(|n| probe.residue(&stopped(&format!("C{n}"))))
        .collect();
    assert_eq!(
        judged,
        [
            Residue::Nothing,
            Residue::Nothing,
            Residue::Nothing,
            Residue::Deferred
        ]
    );
    assert_eq!(host.inspected.lock().unwrap().len(), 3);
    // The disk is still asked once the budget is spent.
    assert_eq!(probe.residue(&stopped("C9")), Residue::StateOnDisk);
}
