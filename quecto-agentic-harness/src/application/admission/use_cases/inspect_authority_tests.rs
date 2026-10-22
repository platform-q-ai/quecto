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
    fn stop(&self, _directory: &Path) -> Result<(), AuthorityAdminError> {
        unreachable!()
    }
}

struct Manager(Result<UnitStatus, String>, Result<bool, String>);

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
    fn is_active(&self) -> Result<bool, String> {
        self.1.clone()
    }
}

fn report(on_demand: bool) -> AuthorityReport {
    AuthorityReport {
        directory: PathBuf::from("/home/me/.quecto/admission"),
        on_demand,
        instance: 1,
        epoch: 1,
        journal_healthy: true,
        live_scopes: 0,
        groups: BTreeMap::new(),
    }
}

fn inspect(unit: Result<UnitStatus, String>, active: Result<bool, String>) -> InspectAuthority {
    InspectAuthority::new(Arc::new(Admin), Arc::new(Manager(unit, active)))
}

const CONFIG: &str = "/home/me/.quecto/config.json";

fn unit_for(config: &str) -> Result<UnitStatus, String> {
    use crate::application::admission::dto::service::InstallServiceRequest;
    Ok(UnitStatus::Present {
        contents: crate::application::admission::use_cases::InstallAuthorityService::unit_contents(
            &InstallServiceRequest {
                binary: PathBuf::from("/usr/bin/quecto"),
                config: PathBuf::from(config),
                directory: PathBuf::from("/ignored"),
                dry_run: false,
            },
        ),
    })
}

/// #2615 review round 2: "stopped" only when the unit serves this config
/// and is inactive; any other installed unit is named without that claim.
#[test]
fn an_on_demand_broker_names_the_unit_it_stands_in_for() {
    let config = Path::new(CONFIG);
    let stood = |unit, active| inspect(unit, active).standing_in(&report(true), config);
    assert_eq!(
        stood(unit_for(CONFIG), Ok(false)),
        Some(StandIn::UnitStopped {
            unit: "quecto-admission-broker.service".into()
        })
    );
    let elsewhere = Some(StandIn::UnitElsewhere {
        unit: "quecto-admission-broker.service".into(),
    });
    assert_eq!(
        stood(unit_for("/elsewhere/config.json"), Ok(false)),
        elsewhere
    );
    assert_eq!(
        stood(unit_for(CONFIG), Ok(true)),
        elsewhere,
        "active: not stopped"
    );
    assert_eq!(stood(unit_for(CONFIG), Err("no bus".into())), elsewhere);
    assert_eq!(stood(Ok(UnitStatus::Absent), Ok(false)), None);
    assert_eq!(stood(Err("unreadable".into()), Ok(false)), None);
    assert_eq!(
        inspect(unit_for(CONFIG), Ok(false)).standing_in(&report(false), config),
        None,
        "the unit serves"
    );
}
