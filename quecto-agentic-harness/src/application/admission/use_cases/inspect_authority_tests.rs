//! `admission-broker status` names an installed unit an on-demand broker is
//! standing in for (#2615 review).
use std::collections::BTreeMap;
use std::path::PathBuf;

use super::*;
use crate::application::admission::dto::authority::ResetReport;
use crate::application::admission::ports::ServiceUnitSpec;

struct Admin;

impl AuthorityAdmin for Admin {
    fn inspect(&self, _directory: &Path) -> Result<AuthorityReport, AuthorityAdminError> {
        unreachable!()
    }
    fn reset(&self, _directory: &Path) -> Result<ResetReport, AuthorityAdminError> {
        unreachable!()
    }
}

struct Manager(Result<UnitStatus, String>);

impl AuthorityServiceManager for Manager {
    fn unit_path(&self) -> PathBuf {
        PathBuf::from("/u/quecto-admission-broker.service")
    }
    fn unit_name(&self) -> String {
        "quecto-admission-broker.service".into()
    }
    fn unit_status(&self) -> Result<UnitStatus, String> {
        self.0.clone()
    }
    fn write_unit(&self, _spec: &ServiceUnitSpec) -> Result<(), String> {
        unreachable!()
    }
    fn remove_unit(&self) -> Result<bool, String> {
        unreachable!()
    }
    fn daemon_reload(&self) -> Result<(), String> {
        unreachable!()
    }
    fn enable_now(&self) -> Result<(), String> {
        unreachable!()
    }
    fn disable_now(&self) -> Result<bool, String> {
        unreachable!()
    }
    fn restart(&self) -> Result<(), String> {
        unreachable!()
    }
}

fn report(on_demand: bool) -> AuthorityReport {
    AuthorityReport {
        directory: PathBuf::from("/home/me/.quecto/admission"),
        on_demand,
        epoch: 1,
        journal_healthy: true,
        live_scopes: 0,
        groups: BTreeMap::new(),
    }
}

fn inspect(unit: Result<UnitStatus, String>) -> InspectAuthority {
    InspectAuthority::new(Arc::new(Admin), Arc::new(Manager(unit)))
}

#[test]
fn an_on_demand_broker_serving_over_an_installed_unit_names_it() {
    let installed = || {
        Ok(UnitStatus::Present {
            contents: "[Unit]\n".into(),
        })
    };
    assert_eq!(
        inspect(installed()).stood_down_unit(&report(true)),
        Some("quecto-admission-broker.service".into())
    );
    assert_eq!(
        inspect(installed()).stood_down_unit(&report(false)),
        None,
        "the unit serves"
    );
    assert_eq!(
        inspect(Ok(UnitStatus::Absent)).stood_down_unit(&report(true)),
        None
    );
    assert_eq!(
        inspect(Err("unreadable".into())).stood_down_unit(&report(true)),
        None
    );
}
