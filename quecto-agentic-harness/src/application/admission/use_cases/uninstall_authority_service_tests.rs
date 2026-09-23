use super::*;
use crate::application::admission::dto::ServiceAction;
use crate::application::admission::ports::{AuthorityServiceManager, ServiceUnitSpec, UnitStatus};
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Default)]
struct FakeManager {
    unit: Mutex<Option<String>>,
    calls: Mutex<Vec<String>>,
    failure: Mutex<Option<&'static str>>,
    disable_result: Mutex<Option<bool>>,
    remove_result: Mutex<Option<bool>>,
}

impl FakeManager {
    fn fail_once(&self, operation: &'static str) {
        *self.failure.lock().unwrap() = Some(operation);
    }

    fn record(&self, operation: &'static str) -> Result<(), String> {
        self.calls.lock().unwrap().push(operation.into());
        let mut failure = self.failure.lock().unwrap();
        if failure.as_ref() == Some(&operation) {
            *failure = None;
            return Err(format!("{operation} failed"));
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
        if self.failure.lock().unwrap().as_ref() == Some(&"unit_status") {
            self.record("unit_status")?;
        }
        Ok(match &*self.unit.lock().unwrap() {
            Some(contents) => UnitStatus::Present {
                contents: contents.clone(),
            },
            None => UnitStatus::Absent,
        })
    }
    fn write_unit(&self, spec: &ServiceUnitSpec) -> Result<(), String> {
        *self.unit.lock().unwrap() = Some(spec.contents.clone());
        Ok(())
    }
    fn remove_unit(&self) -> Result<bool, String> {
        self.record("remove_unit")?;
        if let Some(result) = self.remove_result.lock().unwrap().take() {
            return Ok(result);
        }
        Ok(self.unit.lock().unwrap().take().is_some())
    }
    fn daemon_reload(&self) -> Result<(), String> {
        self.record("daemon_reload")
    }
    fn enable_now(&self) -> Result<(), String> {
        Ok(())
    }
    fn disable_now(&self) -> Result<bool, String> {
        self.record("disable_now")?;
        Ok(self.disable_result.lock().unwrap().take().unwrap_or(true))
    }
    fn restart(&self) -> Result<(), String> {
        self.calls.lock().unwrap().push("restart".into());
        Ok(())
    }
}

fn dir() -> PathBuf {
    PathBuf::from("/home/me/.quecto/admission")
}

#[test]
fn uninstall_disables_and_removes_a_present_unit() {
    let manager = Arc::new(FakeManager::default());
    manager
        .write_unit(&ServiceUnitSpec {
            contents: "unit".into(),
        })
        .unwrap();
    let report = UninstallAuthorityService::new(manager.clone())
        .execute(dir(), false)
        .unwrap();
    assert_eq!(
        *manager.calls.lock().unwrap(),
        vec!["disable_now", "remove_unit", "daemon_reload"]
    );
    assert!(
        report
            .actions
            .iter()
            .any(|a| matches!(a, ServiceAction::DisabledAndStopped { .. }))
    );
    assert!(
        report
            .actions
            .iter()
            .any(|a| matches!(a, ServiceAction::RemovedUnit { .. }))
    );
}

#[test]
fn uninstall_of_an_absent_unit_reports_nothing_to_remove() {
    let manager = Arc::new(FakeManager::default());
    let report = UninstallAuthorityService::new(manager.clone())
        .execute(dir(), false)
        .unwrap();
    assert_eq!(*manager.calls.lock().unwrap(), ["daemon_reload"]);
    assert_eq!(report.actions.len(), 2);
    assert!(matches!(
        report.actions[0],
        ServiceAction::NoUnitToRemove { .. }
    ));
}

#[test]
fn a_failed_disable_preserves_the_unit_and_retry_finishes_in_order() {
    let manager = Arc::new(FakeManager::default());
    manager
        .write_unit(&ServiceUnitSpec {
            contents: "unit".into(),
        })
        .unwrap();
    manager.fail_once("disable_now");
    let use_case = UninstallAuthorityService::new(manager.clone());
    assert_eq!(
        use_case.execute(dir(), false).unwrap_err(),
        "disable_now failed"
    );
    assert_eq!(*manager.calls.lock().unwrap(), ["disable_now"]);
    assert!(matches!(
        manager.unit_status().unwrap(),
        UnitStatus::Present { .. }
    ));
    let report = use_case.execute(dir(), false).unwrap();
    assert_eq!(
        *manager.calls.lock().unwrap(),
        ["disable_now", "disable_now", "remove_unit", "daemon_reload"]
    );
    assert!(matches!(
        report.actions.as_slice(),
        [
            ServiceAction::DisabledAndStopped { .. },
            ServiceAction::RemovedUnit { .. },
            ServiceAction::DaemonReloaded
        ]
    ));
}

#[test]
fn a_failed_remove_retries_without_reloading_early() {
    let manager = Arc::new(FakeManager::default());
    manager
        .write_unit(&ServiceUnitSpec {
            contents: "unit".into(),
        })
        .unwrap();
    manager.fail_once("remove_unit");
    let use_case = UninstallAuthorityService::new(manager.clone());
    assert_eq!(
        use_case.execute(dir(), false).unwrap_err(),
        "remove_unit failed"
    );
    assert_eq!(
        *manager.calls.lock().unwrap(),
        ["disable_now", "remove_unit"]
    );
    assert!(matches!(
        manager.unit_status().unwrap(),
        UnitStatus::Present { .. }
    ));
    let report = use_case.execute(dir(), false).unwrap();
    assert_eq!(
        *manager.calls.lock().unwrap(),
        [
            "disable_now",
            "remove_unit",
            "disable_now",
            "remove_unit",
            "daemon_reload"
        ]
    );
    assert!(matches!(
        report.actions.as_slice(),
        [
            ServiceAction::DisabledAndStopped { .. },
            ServiceAction::RemovedUnit { .. },
            ServiceAction::DaemonReloaded
        ]
    ));
}

#[test]
fn dry_run_plans_present_and_absent_units_without_mutating_them() {
    let manager = Arc::new(FakeManager::default());
    let use_case = UninstallAuthorityService::new(manager.clone());
    let absent = use_case.execute(dir(), true).unwrap();
    assert!(absent.dry_run);
    assert!(
        matches!(absent.actions.as_slice(), [ServiceAction::Planned { description }] if description.contains("nothing to remove"))
    );
    manager
        .write_unit(&ServiceUnitSpec {
            contents: "unit".into(),
        })
        .unwrap();
    let present = use_case.execute(dir(), true).unwrap();
    assert!(present.dry_run);
    assert!(
        matches!(present.actions.as_slice(), [ServiceAction::Planned { description }] if description.contains("disable --now") && description.contains("remove"))
    );
    assert!(manager.calls.lock().unwrap().is_empty());
    assert!(matches!(
        manager.unit_status().unwrap(),
        UnitStatus::Present { .. }
    ));
}

#[test]
fn status_failure_has_no_mutating_calls_and_retry_succeeds() {
    let manager = Arc::new(FakeManager::default());
    manager
        .write_unit(&ServiceUnitSpec {
            contents: "unit".into(),
        })
        .unwrap();
    manager.fail_once("unit_status");
    let use_case = UninstallAuthorityService::new(manager.clone());
    assert_eq!(
        use_case.execute(dir(), false).unwrap_err(),
        "unit_status failed"
    );
    assert_eq!(*manager.calls.lock().unwrap(), ["unit_status"]);
    manager.calls.lock().unwrap().clear();
    assert!(use_case.execute(dir(), false).is_ok());
    assert_eq!(
        *manager.calls.lock().unwrap(),
        ["disable_now", "remove_unit", "daemon_reload"]
    );
}

#[test]
fn reload_failure_follows_removal_and_absent_retry_reloads_without_repeat_mutations() {
    let manager = Arc::new(FakeManager::default());
    manager
        .write_unit(&ServiceUnitSpec {
            contents: "unit".into(),
        })
        .unwrap();
    manager.fail_once("daemon_reload");
    let use_case = UninstallAuthorityService::new(manager.clone());
    assert_eq!(
        use_case.execute(dir(), false).unwrap_err(),
        "daemon_reload failed"
    );
    assert_eq!(
        *manager.calls.lock().unwrap(),
        ["disable_now", "remove_unit", "daemon_reload"]
    );
    assert!(matches!(manager.unit_status().unwrap(), UnitStatus::Absent));
    // A fresh use-case instance must recover after a process restart; another
    // reload failure must remain an error rather than claim convergence.
    manager.fail_once("daemon_reload");
    assert_eq!(
        UninstallAuthorityService::new(manager.clone())
            .execute(dir(), false)
            .unwrap_err(),
        "daemon_reload failed"
    );
    assert_eq!(
        *manager.calls.lock().unwrap(),
        [
            "disable_now",
            "remove_unit",
            "daemon_reload",
            "daemon_reload"
        ]
    );
    let retry = UninstallAuthorityService::new(manager.clone())
        .execute(dir(), false)
        .unwrap();
    assert!(matches!(
        retry.actions.as_slice(),
        [
            ServiceAction::NoUnitToRemove { .. },
            ServiceAction::DaemonReloaded
        ]
    ));
    assert_eq!(
        *manager.calls.lock().unwrap(),
        [
            "disable_now",
            "remove_unit",
            "daemon_reload",
            "daemon_reload",
            "daemon_reload"
        ]
    );
}

#[test]
fn false_disable_and_remove_results_are_reported_in_order() {
    let manager = Arc::new(FakeManager::default());
    manager
        .write_unit(&ServiceUnitSpec {
            contents: "unit".into(),
        })
        .unwrap();
    *manager.disable_result.lock().unwrap() = Some(false);
    *manager.remove_result.lock().unwrap() = Some(false);
    let report = UninstallAuthorityService::new(manager.clone())
        .execute(dir(), false)
        .unwrap();
    assert!(matches!(
        report.actions.as_slice(),
        [
            ServiceAction::NothingToDisable { .. },
            ServiceAction::NoUnitToRemove { .. },
            ServiceAction::DaemonReloaded
        ]
    ));
    assert_eq!(
        *manager.calls.lock().unwrap(),
        ["disable_now", "remove_unit", "daemon_reload"]
    );
    assert!(matches!(
        manager.unit_status().unwrap(),
        UnitStatus::Present { .. }
    ));
}
