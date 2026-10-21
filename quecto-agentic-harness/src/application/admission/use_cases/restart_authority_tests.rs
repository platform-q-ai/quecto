//! `/admission restart` (#2615) over fakes: the installed unit restarts a
//! broker it serves; otherwise a running broker is stopped and one is started
//! on demand; a broker that was not running is started.
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::*;
use crate::application::admission::dto::authority::{
    AuthorityAdminError, AuthorityReport, ResetReport,
};
use crate::application::admission::dto::negotiation::{RestartError, RestartReport};
use crate::application::admission::dto::service::InstallServiceRequest;
use crate::application::admission::ports::{LaunchState, LaunchedAuthority, ServiceUnitSpec};
use crate::application::admission::use_cases::InstallAuthorityService;

#[derive(Default)]
struct Calls(Mutex<Vec<String>>);

impl Calls {
    fn push(&self, call: impl Into<String>) {
        self.0.lock().unwrap().push(call.into());
    }
    fn all(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

/// Probes answer from a script (unreachable once it runs out); launches
/// and pauses are recorded.
struct Launcher {
    probes: Mutex<VecDeque<bool>>,
    calls: Arc<Calls>,
}

struct Running;

impl LaunchedAuthority for Running {
    fn state(&mut self) -> LaunchState {
        LaunchState::Running
    }
}

impl AuthorityLauncher for Launcher {
    fn probe(&self, _directory: &Path) -> Result<(), String> {
        match self.probes.lock().unwrap().pop_front().unwrap_or(false) {
            true => Ok(()),
            false => Err("connection refused".into()),
        }
    }
    fn launch(&self, _launch: &AuthorityLaunch) -> Result<Box<dyn LaunchedAuthority>, String> {
        self.calls.push("launch");
        Ok(Box::new(Running))
    }
    fn pause(&self, _attempt: u32) {}
}

struct Admin {
    calls: Arc<Calls>,
    stop: Result<(), AuthorityAdminError>,
}

impl AuthorityAdmin for Admin {
    fn inspect(&self, _directory: &Path) -> Result<AuthorityReport, AuthorityAdminError> {
        unreachable!("restart never inspects")
    }
    fn reset(&self, _directory: &Path) -> Result<ResetReport, AuthorityAdminError> {
        unreachable!("restart never resets")
    }
    fn stop(&self, _directory: &Path) -> Result<(), AuthorityAdminError> {
        self.calls.push("stop");
        self.stop.clone()
    }
}

struct Manager {
    unit: Option<String>,
    calls: Arc<Calls>,
}

impl AuthorityServiceManager for Manager {
    fn unit_path(&self) -> PathBuf {
        PathBuf::from("/u/quecto-admission-broker.service")
    }
    fn unit_name(&self) -> String {
        "quecto-admission-broker.service".into()
    }
    fn unit_status(&self) -> Result<UnitStatus, String> {
        Ok(match &self.unit {
            Some(contents) => UnitStatus::Present {
                contents: contents.clone(),
            },
            None => UnitStatus::Absent,
        })
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
        self.calls.push("systemctl restart");
        Ok(())
    }
}

/// The session's binding: how it joined, and whether a rejoin succeeds.
struct Session {
    joined: Option<JoinedAuthority>,
    rejoin: Result<(), String>,
    calls: Arc<Calls>,
}

impl AuthoritySession for Session {
    fn joined(&self) -> Option<JoinedAuthority> {
        self.joined.clone()
    }
    fn rejoin(&self) -> Result<(), String> {
        self.calls.push("rejoin");
        self.rejoin.clone()
    }
    fn health(&self) -> String {
        "connected".into()
    }
}

fn launch() -> AuthorityLaunch {
    AuthorityLaunch {
        directory: PathBuf::from("/home/me/.quecto/admission"),
        config: PathBuf::from("/home/me/.quecto/config.json"),
        base_dir: PathBuf::from("/home/me/.quecto"),
    }
}

fn unit_for(config: &str) -> String {
    InstallAuthorityService::unit_contents(&InstallServiceRequest {
        binary: PathBuf::from("/usr/bin/quecto"),
        config: PathBuf::from(config),
        directory: PathBuf::from("/ignored"),
        dry_run: false,
    })
}

fn restart(
    probes: &[bool],
    unit: Option<String>,
    stop: Result<(), AuthorityAdminError>,
) -> (Result<RestartOutcome, String>, Vec<String>) {
    let (outcome, calls) = restart_as(
        Some(JoinedAuthority::Root(launch())),
        Ok(()),
        probes,
        unit,
        stop,
    );
    let outcome = match outcome {
        Ok(report) => {
            assert_eq!(report.health, "connected");
            assert_eq!(calls.last().map(String::as_str), Some("rejoin"));
            Ok(report.outcome)
        }
        Err(error) => Err(format!("{error:?}")),
    };
    let calls = calls.into_iter().filter(|call| call != "rejoin").collect();
    (outcome, calls)
}

fn restart_as(
    joined: Option<JoinedAuthority>,
    rejoin: Result<(), String>,
    probes: &[bool],
    unit: Option<String>,
    stop: Result<(), AuthorityAdminError>,
) -> (Result<RestartReport, RestartError>, Vec<String>) {
    let calls = Arc::new(Calls::default());
    let launcher = Arc::new(Launcher {
        probes: Mutex::new(probes.iter().copied().collect()),
        calls: calls.clone(),
    });
    let use_case = RestartAuthority::new(
        Arc::new(Admin {
            calls: calls.clone(),
            stop,
        }),
        Arc::new(Manager {
            unit,
            calls: calls.clone(),
        }),
        launcher.clone(),
        Arc::new(NegotiateAuthority::new(launcher)),
        Arc::new(Session {
            joined,
            rejoin,
            calls: calls.clone(),
        }),
    );
    (use_case.execute(), calls.all())
}

#[test]
fn the_installed_unit_serving_this_config_restarts_its_broker() {
    let (outcome, calls) = restart(
        &[false, true],
        Some(unit_for("/home/me/.quecto/config.json")),
        Ok(()),
    );
    assert_eq!(outcome, Ok(RestartOutcome::ServiceRestarted));
    assert_eq!(calls, ["systemctl restart"], "never stopped or launched");
}

#[test]
fn a_unit_that_never_answers_after_its_restart_is_reported() {
    let (outcome, _) = restart(&[], Some(unit_for("/home/me/.quecto/config.json")), Ok(()));
    let error = outcome.unwrap_err();
    assert!(
        error.contains("quecto-admission-broker.service was restarted"),
        "{error}"
    );
}

#[test]
fn a_running_broker_without_a_unit_is_stopped_and_started_on_demand() {
    // Running, still up after the stop, gone, (ensure) unreachable, answers.
    let (outcome, calls) = restart(&[true, true, false, false, true], None, Ok(()));
    assert_eq!(outcome, Ok(RestartOutcome::Restarted));
    assert_eq!(calls, ["stop", "launch"]);
}

#[test]
fn a_unit_for_another_config_is_not_this_brokers_manager() {
    let (outcome, calls) = restart(
        &[true, false, false, true],
        Some(unit_for("/elsewhere/config.json")),
        Ok(()),
    );
    assert_eq!(outcome, Ok(RestartOutcome::Restarted));
    assert_eq!(calls, ["stop", "launch"]);
}

#[test]
fn a_broker_that_was_not_running_is_started() {
    let (outcome, calls) = restart(&[false, false, true], None, Ok(()));
    assert_eq!(outcome, Ok(RestartOutcome::Started));
    assert_eq!(calls, ["launch"], "nothing to stop");
}

#[test]
fn a_refused_stop_or_a_broker_that_never_stops_is_reported() {
    let refused = AuthorityAdminError::Failed {
        directory: PathBuf::from("/home/me/.quecto/admission"),
        reason: "unauthorized".into(),
    };
    let (outcome, calls) = restart(&[true], None, Err(refused));
    assert!(outcome.unwrap_err().contains("unauthorized"));
    assert_eq!(calls, ["stop"], "nothing started over a refused stop");
    let (outcome, calls) = restart(&[true; 64], None, Ok(()));
    assert!(outcome.unwrap_err().contains("did not stop"));
    assert_eq!(calls, ["stop"]);
}

#[test]
fn only_a_root_restarts_the_broker() {
    let (outcome, calls) = restart_as(None, Ok(()), &[true], None, Ok(()));
    assert_eq!(outcome, Err(RestartError::NotConfigured));
    assert!(calls.is_empty(), "nothing touched: {calls:?}");
    let (outcome, calls) = restart_as(Some(JoinedAuthority::Child), Ok(()), &[true], None, Ok(()));
    assert_eq!(outcome, Err(RestartError::Child));
    assert!(calls.is_empty(), "nothing touched: {calls:?}");
}

#[test]
fn a_broker_back_that_the_session_cannot_rejoin_says_both() {
    let (outcome, calls) = restart_as(
        Some(JoinedAuthority::Root(launch())),
        Err("restart required".into()),
        &[false, false, true],
        None,
        Ok(()),
    );
    assert_eq!(
        outcome,
        Err(RestartError::Rejoin {
            outcome: RestartOutcome::Started,
            reason: "restart required".into(),
        })
    );
    assert_eq!(calls, ["launch", "rejoin"]);
}

/// A launcher whose probe holds a restart until the test releases it, so
/// two restarts overlap; bounded, so a restart that never probes fails the
/// test rather than hanging it.
struct Holding {
    calls: Arc<Calls>,
    entered: Mutex<std::sync::mpsc::Sender<()>>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
}

const HOLD: std::time::Duration = std::time::Duration::from_secs(10);

impl AuthorityLauncher for Holding {
    fn probe(&self, _directory: &Path) -> Result<(), String> {
        let _ = self.entered.lock().unwrap().send(());
        let _ = self.release.lock().unwrap().recv_timeout(HOLD);
        Err("connection refused".into())
    }
    fn launch(&self, _launch: &AuthorityLaunch) -> Result<Box<dyn LaunchedAuthority>, String> {
        self.calls.push("launch");
        Err("held".into())
    }
    fn pause(&self, _attempt: u32) {}
}

#[test]
fn one_restart_runs_at_a_time() {
    let calls = Arc::new(Calls::default());
    let (entered_tx, entered) = std::sync::mpsc::channel();
    let (release, release_rx) = std::sync::mpsc::channel();
    let use_case = Arc::new(RestartAuthority::new(
        Arc::new(Admin {
            calls: calls.clone(),
            stop: Ok(()),
        }),
        Arc::new(Manager {
            unit: None,
            calls: calls.clone(),
        }),
        Arc::new(Holding {
            calls: calls.clone(),
            entered: Mutex::new(entered_tx),
            release: Mutex::new(release_rx),
        }),
        Arc::new(NegotiateAuthority::new(Arc::new(Launcher {
            probes: Mutex::new(VecDeque::new()),
            calls: calls.clone(),
        }))),
        Arc::new(Session {
            joined: Some(JoinedAuthority::Root(launch())),
            rejoin: Ok(()),
            calls: calls.clone(),
        }),
    ));
    let first = std::thread::spawn({
        let use_case = use_case.clone();
        move || use_case.execute()
    });
    entered
        .recv_timeout(HOLD)
        .expect("the first restart reached its probe");
    assert_eq!(use_case.execute(), Err(RestartError::InProgress));
    release.send(()).unwrap();
    assert!(first.join().unwrap().is_err(), "the held restart fails on");
    // Released either way: a later restart runs again.
    let again = std::thread::spawn({
        let use_case = use_case.clone();
        move || use_case.execute()
    });
    entered
        .recv_timeout(HOLD)
        .expect("a later restart runs again");
    release.send(()).unwrap();
    assert_ne!(again.join().unwrap(), Err(RestartError::InProgress));
}
