use std::path::Path;
use std::sync::Mutex;

use super::{
    NO_ENVIRONMENT_DIR, Residue, ResidueProbe, SLOW_INSPECT_MILLIS, STATE_GONE_CONTAINER_RUNNING,
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
    /// What one inspect takes on the fake clock.
    cost_millis: u64,
    /// Time that passes outside the probe (the caller's own inspects).
    outside_millis: Mutex<u64>,
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
    fn inspect_clock_millis(&self) -> u64 {
        *self.outside_millis.lock().unwrap()
            + self.cost_millis * self.inspected.lock().unwrap().len() as u64
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
        cost_millis: 0,
        outside_millis: Mutex::default(),
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
    // Each inspect takes 3 s; the budget is 10 s: after four inspects
    // (12 s spent) the fifth record waits.
    let host = Host {
        cost_millis: 3_000,
        ..host(vec!["/state/env-C9"], vec![])
    };
    let mut probe = ResidueProbe::with_budget(&host, 10_000);
    let judged: Vec<Residue> = (1..=5)
        .map(|n| probe.residue(&stopped(&format!("C{n}"))))
        .collect();
    assert_eq!(
        judged[..4],
        [
            Residue::Nothing,
            Residue::Nothing,
            Residue::Nothing,
            Residue::Nothing
        ]
    );
    assert_eq!(judged[4], Residue::Deferred);
    assert_eq!(host.inspected.lock().unwrap().len(), 4);
    // The disk is still asked once the budget is spent.
    assert_eq!(probe.residue(&stopped("C9")), Residue::StateOnDisk);
    // A new probe has its own budget.
    assert_eq!(
        ResidueProbe::with_budget(&host, 10_000).residue(&stopped("C6")),
        Residue::Nothing
    );
}

#[test]
fn time_outside_the_probes_own_inspects_is_not_its_to_count() {
    let host = host(vec![], vec![]);
    let mut probe = ResidueProbe::with_budget(&host, 10_000);
    // The caller spends a minute inspecting live records meanwhile.
    *host.outside_millis.lock().unwrap() += 60_000;
    assert_eq!(probe.residue(&stopped("C1")), Residue::Nothing);
}

#[test]
fn a_slow_inspect_trips_the_breaker_for_its_runtime_only() {
    // A 4 s inspect (the script bound is 5 s): its runtime is hanging, so
    // its other records wait; another runtime's record is still judged.
    let host = Host {
        cost_millis: SLOW_INSPECT_MILLIS,
        ..host(vec![], vec![])
    };
    let mut probe = ResidueProbe::new(&host);
    assert_eq!(probe.residue(&stopped("C1")), Residue::Nothing);
    assert_eq!(probe.residue(&stopped("C2")), Residue::Deferred);
    let mut other_runtime = stopped("C3");
    other_runtime.retained_inspect_argv = vec!["other-inspect".into()];
    assert_eq!(probe.residue(&other_runtime), Residue::Nothing);
    assert_eq!(*host.inspected.lock().unwrap(), ["C1", "C3"]);
}
