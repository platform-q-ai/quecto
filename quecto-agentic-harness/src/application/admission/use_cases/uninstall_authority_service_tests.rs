use super::*;
use crate::application::admission::dto::ServiceAction;
use crate::application::admission::ports::{AuthorityServiceManager, ServiceUnitSpec, UnitStatus};
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Default)]
struct FakeManager {
    unit: Mutex<Option<String>>,
    calls: Mutex<Vec<String>>,
}

impl AuthorityServiceManager for FakeManager {
    fn unit_path(&self) -> PathBuf {
        PathBuf::from("/home/me/.config/systemd/user/quecto-admission-broker.service")
    }
    fn unit_name(&self) -> String {
        "quecto-admission-broker.service".into()
    }
    fn unit_status(&self) -> Result<UnitStatus, String> {
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
        self.calls.lock().unwrap().push("remove_unit".into());
        Ok(self.unit.lock().unwrap().take().is_some())
    }
    fn daemon_reload(&self) -> Result<(), String> {
        self.calls.lock().unwrap().push("daemon_reload".into());
        Ok(())
    }
    fn enable_now(&self) -> Result<(), String> {
        Ok(())
    }
    fn disable_now(&self) -> Result<bool, String> {
        self.calls.lock().unwrap().push("disable_now".into());
        Ok(true)
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
    assert!(manager.calls.lock().unwrap().is_empty());
    assert_eq!(report.actions.len(), 1);
    assert!(matches!(
        report.actions[0],
        ServiceAction::NoUnitToRemove { .. }
    ));
}
