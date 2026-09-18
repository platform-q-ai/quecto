use super::*;
use crate::application::admission::dto::{InstallServiceRequest, ServiceAction};
use crate::application::admission::ports::{AuthorityServiceManager, ServiceUnitSpec, UnitStatus};
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Default)]
struct FakeManager {
    unit: Mutex<Option<String>>,
    calls: Mutex<Vec<String>>,
    enable_fails: bool,
}

impl FakeManager {
    fn record(&self, call: &str) {
        self.calls.lock().unwrap().push(call.to_string());
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
        Ok(match &*self.unit.lock().unwrap() {
            Some(contents) => UnitStatus::Present {
                contents: contents.clone(),
            },
            None => UnitStatus::Absent,
        })
    }
    fn write_unit(&self, spec: &ServiceUnitSpec) -> Result<(), String> {
        self.record("write_unit");
        *self.unit.lock().unwrap() = Some(spec.contents.clone());
        Ok(())
    }
    fn remove_unit(&self) -> Result<bool, String> {
        self.record("remove_unit");
        Ok(self.unit.lock().unwrap().take().is_some())
    }
    fn daemon_reload(&self) -> Result<(), String> {
        self.record("daemon_reload");
        Ok(())
    }
    fn enable_now(&self) -> Result<(), String> {
        self.record("enable_now");
        if self.enable_fails {
            Err("systemctl enable failed".into())
        } else {
            Ok(())
        }
    }
    fn disable_now(&self) -> Result<bool, String> {
        self.record("disable_now");
        Ok(true)
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
    assert!(contents.contains(
        "ExecStart=/usr/bin/quecto admission-broker run --config /home/me/.quecto/config.json"
    ));
    assert!(contents.contains("Restart=on-failure"));
    assert!(contents.contains("WantedBy=default.target"));
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
