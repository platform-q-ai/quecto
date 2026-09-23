use super::*;
use crate::application::admission::dto::{InstallServiceRequest, ServiceAction};
use crate::application::admission::ports::{AuthorityServiceManager, ServiceUnitSpec, UnitStatus};
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Default)]
struct FakeManager {
    unit: Mutex<Option<String>>,
    calls: Mutex<Vec<String>>,
    fail_on: Mutex<Option<&'static str>>,
    status_calls: Mutex<usize>,
}

impl FakeManager {
    fn record(&self, call: &'static str) -> Result<(), String> {
        self.calls.lock().unwrap().push(call.to_string());
        if *self.fail_on.lock().unwrap() == Some(call) {
            return Err(format!("{call} failed"));
        }
        Ok(())
    }
}

impl AuthorityServiceManager for FakeManager {
    fn unit_path(&self) -> PathBuf {
        PathBuf::from("/home/me/.config/systemd/user/quecto-admission-broker.service")
    }
    fn unit_name(&self) -> String {
        "quecto-admission-broker.service".into()
    }
    fn unit_status(&self) -> Result<UnitStatus, String> {
        *self.status_calls.lock().unwrap() += 1;
        if *self.fail_on.lock().unwrap() == Some("unit_status") {
            return Err("unit_status failed".into());
        }
        Ok(match &*self.unit.lock().unwrap() {
            Some(contents) => UnitStatus::Present {
                contents: contents.clone(),
            },
            None => UnitStatus::Absent,
        })
    }
    fn write_unit(&self, spec: &ServiceUnitSpec) -> Result<(), String> {
        self.record("write_unit")?;
        *self.unit.lock().unwrap() = Some(spec.contents.clone());
        Ok(())
    }
    fn remove_unit(&self) -> Result<bool, String> {
        self.record("remove_unit")?;
        Ok(self.unit.lock().unwrap().take().is_some())
    }
    fn daemon_reload(&self) -> Result<(), String> {
        self.record("daemon_reload")?;
        Ok(())
    }
    fn enable_now(&self) -> Result<(), String> {
        self.record("enable_now")
    }
    fn disable_now(&self) -> Result<bool, String> {
        self.record("disable_now")?;
        Ok(true)
    }
    fn restart(&self) -> Result<(), String> {
        self.record("restart")
    }
}

fn request(dry_run: bool) -> InstallServiceRequest {
    InstallServiceRequest {
        binary: PathBuf::from("/usr/bin/quecto"),
        config: PathBuf::from("/home/me/.quecto/config.json"),
        directory: PathBuf::from("/home/me/.quecto/admission"),
        dry_run,
    }
}

#[test]
fn the_unit_runs_the_broker_with_an_explicit_config_and_restarts() {
    let contents = InstallAuthorityService::unit_contents(&request(false));
    // Paths are quoted so a space in $HOME or the install prefix survives.
    assert!(
        contents.contains(
            "ExecStart=\"/usr/bin/quecto\" admission-broker run --config \"/home/me/.quecto/config.json\""
        ),
        "{contents}"
    );
    assert!(contents.contains("Restart=on-failure"));
    assert!(contents.contains("WantedBy=default.target"));
}

#[test]
fn the_unit_has_no_ordering_cycle_and_never_loops_on_a_busy_lock() {
    let contents = InstallAuthorityService::unit_contents(&request(false));
    // `WantedBy=default.target` already orders default.target *after* the
    // service; `After=default.target` would close a cycle and systemd would
    // drop the job at login (H3). Order after basic.target instead.
    assert!(!contents.contains("After=default.target"), "{contents}");
    assert!(contents.contains("After=basic.target"), "{contents}");
    // Exit 3 is `Busy`: another broker holds the directory lock. Restarting
    // would spin against it every RestartSec.
    assert!(
        contents.contains("RestartPreventExitStatus=3"),
        "{contents}"
    );
}

#[test]
fn a_changed_unit_is_rewritten_and_the_service_restarted() {
    let manager = Arc::new(FakeManager::default());
    let uc = InstallAuthorityService::new(manager.clone());
    uc.execute(request(false)).unwrap();
    manager.calls.lock().unwrap().clear();
    let mut changed = request(false);
    changed.binary = PathBuf::from("/opt/new/quecto");
    let report = uc.execute(changed).unwrap();
    // A running broker keeps serving the old unit until restarted;
    // `enable --now` alone would leave it on the stale ExecStart.
    assert_eq!(
        *manager.calls.lock().unwrap(),
        vec!["write_unit", "daemon_reload", "enable_now", "restart"]
    );
    assert!(matches!(report.actions[0], ServiceAction::WroteUnit { .. }));
    assert!(
        report
            .actions
            .iter()
            .any(|a| matches!(a, ServiceAction::Restarted { .. }))
    );
    // A dry run of a further change plans the restart too.
    let mut planned = request(true);
    planned.binary = PathBuf::from("/opt/newer/quecto");
    let report = uc.execute(planned).unwrap();
    assert!(report.actions.iter().any(
        |a| matches!(a, ServiceAction::Planned { description } if description.contains("restart"))
    ));
}

#[test]
fn a_first_install_writes_reloads_and_enables() {
    let manager = Arc::new(FakeManager::default());
    let report = InstallAuthorityService::new(manager.clone())
        .execute(request(false))
        .unwrap();
    assert_eq!(
        *manager.calls.lock().unwrap(),
        vec!["write_unit", "daemon_reload", "enable_now"]
    );
    assert!(matches!(report.actions[0], ServiceAction::WroteUnit { .. }));
    assert!(
        report
            .actions
            .iter()
            .any(|a| matches!(a, ServiceAction::EnabledAndStarted { .. }))
    );
    assert_eq!(
        report.directory,
        PathBuf::from("/home/me/.quecto/admission")
    );
}

#[test]
fn a_second_install_does_not_rewrite_the_unchanged_unit() {
    let manager = Arc::new(FakeManager::default());
    let uc = InstallAuthorityService::new(manager.clone());
    uc.execute(request(false)).unwrap();
    manager.calls.lock().unwrap().clear();
    let report = uc.execute(request(false)).unwrap();
    // Idempotent: no rewrite, but reload+enable still converge state.
    assert_eq!(
        *manager.calls.lock().unwrap(),
        vec!["daemon_reload", "enable_now"]
    );
    assert!(matches!(
        report.actions[0],
        ServiceAction::UnitUnchanged { .. }
    ));
}

#[test]
fn a_dry_run_touches_nothing() {
    let manager = Arc::new(FakeManager::default());
    let report = InstallAuthorityService::new(manager.clone())
        .execute(request(true))
        .unwrap();
    assert!(report.dry_run);
    assert!(manager.calls.lock().unwrap().is_empty());
    assert!(
        report
            .actions
            .iter()
            .all(|a| matches!(a, ServiceAction::Planned { .. }))
    );
}

#[test]
fn each_failed_install_step_stops_later_actions_and_a_retry_converges() {
    for (failure, first_calls, retry_calls) in [
        (
            "write_unit",
            vec!["write_unit"],
            vec!["write_unit", "daemon_reload", "enable_now"],
        ),
        (
            "daemon_reload",
            vec!["write_unit", "daemon_reload"],
            vec!["daemon_reload", "enable_now"],
        ),
        (
            "enable_now",
            vec!["write_unit", "daemon_reload", "enable_now"],
            vec!["daemon_reload", "enable_now"],
        ),
    ] {
        let manager = Arc::new(FakeManager::default());
        *manager.fail_on.lock().unwrap() = Some(failure);
        let use_case = InstallAuthorityService::new(manager.clone());
        assert_eq!(
            use_case.execute(request(false)).unwrap_err(),
            format!("{failure} failed")
        );
        assert_eq!(
            *manager.calls.lock().unwrap(),
            first_calls,
            "failed {failure}"
        );
        *manager.fail_on.lock().unwrap() = None;
        manager.calls.lock().unwrap().clear();
        let report = use_case.execute(request(false)).unwrap();
        assert_eq!(
            *manager.calls.lock().unwrap(),
            retry_calls,
            "retry after {failure}"
        );
        assert!(matches!(
            report.actions.last(),
            Some(ServiceAction::EnabledAndStarted { .. })
        ));
    }
}

#[test]
fn failed_restart_of_changed_unit_propagates_and_retry_reloads_existing_unit() {
    let manager = Arc::new(FakeManager::default());
    let use_case = InstallAuthorityService::new(manager.clone());
    use_case.execute(request(false)).unwrap();
    manager.calls.lock().unwrap().clear();
    *manager.fail_on.lock().unwrap() = Some("restart");
    let mut changed = request(false);
    changed.binary = PathBuf::from("/opt/new/quecto");
    assert_eq!(
        use_case.execute(changed.clone()).unwrap_err(),
        "restart failed"
    );
    assert_eq!(
        *manager.calls.lock().unwrap(),
        ["write_unit", "daemon_reload", "enable_now", "restart"]
    );
    *manager.fail_on.lock().unwrap() = None;
    manager.calls.lock().unwrap().clear();
    let report = use_case.execute(changed).unwrap();
    // The persisted unit already matches; current policy cannot distinguish
    // a failed restart from a successful one without service-state inspection.
    assert_eq!(
        *manager.calls.lock().unwrap(),
        ["daemon_reload", "enable_now"]
    );
    assert!(matches!(
        report.actions.last(),
        Some(ServiceAction::EnabledAndStarted { .. })
    ));
}

#[test]
fn dry_run_of_changed_unit_plans_in_order_without_mutation_even_if_operations_fail() {
    let manager = Arc::new(FakeManager::default());
    let use_case = InstallAuthorityService::new(manager.clone());
    use_case.execute(request(false)).unwrap();
    manager.calls.lock().unwrap().clear();
    *manager.fail_on.lock().unwrap() = Some("write_unit");
    let original = manager.unit.lock().unwrap().clone();
    let mut changed = request(true);
    changed.binary = PathBuf::from("/opt/new/quecto");
    let report = use_case.execute(changed).unwrap();
    let plans: Vec<_> = report
        .actions
        .iter()
        .map(|action| match action {
            ServiceAction::Planned { description } => description.as_str(),
            other => panic!("dry run produced real action: {other:?}"),
        })
        .collect();
    assert_eq!(plans.len(), 3);
    assert!(plans[0].contains("rewrite unit"), "{plans:?}");
    assert!(
        plans[1].contains("daemon-reload && enable --now"),
        "{plans:?}"
    );
    assert!(plans[2].contains("restart"), "{plans:?}");
    assert!(manager.calls.lock().unwrap().is_empty());
    assert_eq!(*manager.unit.lock().unwrap(), original);
}

#[test]
fn status_failure_aborts_install_and_dry_run_before_any_mutation_or_plan() {
    for dry_run in [false, true] {
        let manager = Arc::new(FakeManager::default());
        *manager.fail_on.lock().unwrap() = Some("unit_status");
        let use_case = InstallAuthorityService::new(manager.clone());
        assert_eq!(
            use_case.execute(request(dry_run)).unwrap_err(),
            "unit_status failed"
        );
        assert_eq!(*manager.status_calls.lock().unwrap(), 1);
        assert!(manager.calls.lock().unwrap().is_empty());
        assert!(manager.unit.lock().unwrap().is_none());
        *manager.fail_on.lock().unwrap() = None;
        let report = use_case.execute(request(dry_run)).unwrap();
        assert_eq!(*manager.status_calls.lock().unwrap(), 2);
        if dry_run {
            assert!(manager.calls.lock().unwrap().is_empty());
            assert!(manager.unit.lock().unwrap().is_none());
            assert!(
                report
                    .actions
                    .iter()
                    .all(|a| matches!(a, ServiceAction::Planned { .. }))
            );
        } else {
            assert_eq!(
                *manager.calls.lock().unwrap(),
                ["write_unit", "daemon_reload", "enable_now"]
            );
        }
    }
}
