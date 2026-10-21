//! `/admission restart` (#2615) over a simulated host: one broker at most
//! serves the directory (the singleton lock); the installed unit starts one
//! only when nothing holds the lock (else it exits Busy and stays stopped);
//! an on-demand start launches one; a stop ends whatever serves. A restart
//! succeeds only when a new broker (another instance) answers.
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::*;
use crate::application::admission::dto::authority::{
    AuthorityAdminError, AuthorityReport, ResetReport,
};
use crate::application::admission::dto::negotiation::{Restart, RestartError, RestartReport};
use crate::application::admission::dto::service::InstallServiceRequest;
use crate::application::admission::ports::{LaunchState, LaunchedAuthority, ServiceUnitSpec};
use crate::application::admission::use_cases::InstallAuthorityService;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Broker {
    instance: u64,
    on_demand: bool,
}

/// The simulated host.
#[derive(Default)]
struct Host {
    serving: Option<Broker>,
    unit_active: bool,
    unit_installed: Option<String>,
    live_scopes: usize,
    next_instance: u64,
    stop_refused: bool,
    stop_ignored: bool,
    unit_never_serves: bool,
    /// The broker accepts but never answers its owner (SIGSTOP).
    wedged: bool,
    /// `systemctl restart` leaves the old broker serving.
    restart_noop: bool,
    /// The broker answers its status with an error.
    status_fails: bool,
    calls: Vec<String>,
}

impl Host {
    fn start(&mut self, on_demand: bool) -> u64 {
        self.next_instance += 1;
        self.serving = Some(Broker {
            instance: self.next_instance,
            on_demand,
        });
        self.next_instance
    }
}

type Shared = Arc<Mutex<Host>>;

struct Launcher(Shared);
struct Admin(Shared);
struct Manager(Shared);
struct Session {
    joined: Option<JoinedAuthority>,
    rejoin: Result<(), String>,
    host: Shared,
}
struct Running;

impl LaunchedAuthority for Running {
    fn state(&mut self) -> LaunchState {
        LaunchState::Running
    }
}

impl AuthorityLauncher for Launcher {
    fn probe(&self, _directory: &Path) -> Result<(), String> {
        match self.0.lock().unwrap().serving {
            Some(_) => Ok(()),
            None => Err("connection refused".into()),
        }
    }
    fn launch(&self, _launch: &AuthorityLaunch) -> Result<Box<dyn LaunchedAuthority>, String> {
        let mut host = self.0.lock().unwrap();
        host.calls.push("launch".into());
        if host.serving.is_none() {
            host.start(true);
        }
        Ok(Box::new(Running))
    }
    fn pause(&self, _attempt: u32) {}
}

impl AuthorityAdmin for Admin {
    fn inspect(&self, directory: &Path) -> Result<AuthorityReport, AuthorityAdminError> {
        let host = self.0.lock().unwrap();
        if host.wedged {
            return Err(AuthorityAdminError::Unanswered {
                directory: directory.to_path_buf(),
                bound_secs: 5,
            });
        }
        if host.status_fails {
            return Err(AuthorityAdminError::Failed {
                directory: directory.to_path_buf(),
                reason: "protocol violation".into(),
            });
        }
        match host.serving {
            Some(broker) => Ok(AuthorityReport {
                directory: directory.to_path_buf(),
                on_demand: broker.on_demand,
                instance: broker.instance,
                epoch: 1,
                journal_healthy: true,
                live_scopes: host.live_scopes,
                groups: Default::default(),
            }),
            None => Err(AuthorityAdminError::NotRunning {
                directory: directory.to_path_buf(),
                reason: "connection refused".into(),
            }),
        }
    }
    fn reset(&self, _directory: &Path) -> Result<ResetReport, AuthorityAdminError> {
        unreachable!("restart never resets")
    }
    fn stop(&self, directory: &Path) -> Result<(), AuthorityAdminError> {
        let mut host = self.0.lock().unwrap();
        host.calls.push("stop".into());
        if host.stop_refused {
            return Err(AuthorityAdminError::Failed {
                directory: directory.to_path_buf(),
                reason: "unauthorized".into(),
            });
        }
        if !host.stop_ignored {
            host.serving = None;
            host.unit_active = false;
        }
        Ok(())
    }
}

impl AuthorityServiceManager for Manager {
    fn unit_path(&self) -> PathBuf {
        PathBuf::from("/u/quecto-admission-broker.service")
    }
    fn unit_name(&self) -> String {
        "quecto-admission-broker.service".into()
    }
    fn unit_status(&self) -> Result<UnitStatus, String> {
        Ok(match &self.0.lock().unwrap().unit_installed {
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
        let mut host = self.0.lock().unwrap();
        host.calls.push("systemctl restart".into());
        if host.restart_noop {
            return Ok(());
        }
        // Restarting the unit stops its own broker (never another's) first.
        if host.unit_active {
            host.wedged = false;
            if host.serving.is_some_and(|broker| !broker.on_demand) {
                host.serving = None;
            }
        }
        match (host.serving, host.unit_never_serves) {
            // The lock is held by another broker: the unit exits Busy.
            (Some(_), _) | (None, true) => host.unit_active = false,
            (None, false) => {
                host.start(false);
                host.unit_active = true;
            }
        }
        Ok(())
    }
    fn is_active(&self) -> Result<bool, String> {
        Ok(self.0.lock().unwrap().unit_active)
    }
}

impl AuthoritySession for Session {
    fn joined(&self) -> Option<JoinedAuthority> {
        self.joined.clone()
    }
    fn rejoin(&self) -> Result<(), String> {
        self.host.lock().unwrap().calls.push("rejoin".into());
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

fn our_unit() -> Option<String> {
    Some(unit_for("/home/me/.quecto/config.json"))
}

fn use_case(
    host: &Shared,
    joined: Option<JoinedAuthority>,
    rejoin: Result<(), String>,
) -> RestartAuthority {
    let launcher = Arc::new(Launcher(host.clone()));
    RestartAuthority::new(
        Arc::new(Admin(host.clone())),
        Arc::new(Manager(host.clone())),
        launcher.clone(),
        Arc::new(NegotiateAuthority::new(launcher)),
        Arc::new(Session {
            joined,
            rejoin,
            host: host.clone(),
        }),
    )
}

fn root_restart(host: &Shared, confirmed: Option<usize>) -> Result<Restart, RestartError> {
    use_case(host, Some(JoinedAuthority::Root(launch())), Ok(())).execute(confirmed)
}

fn host(setup: impl FnOnce(&mut Host)) -> Shared {
    let mut host = Host {
        live_scopes: 1,
        ..Host::default()
    };
    setup(&mut host);
    Arc::new(Mutex::new(host))
}

fn calls(host: &Shared) -> Vec<String> {
    host.lock().unwrap().calls.clone()
}

fn done(outcome: RestartOutcome) -> Result<Restart, RestartError> {
    Ok(Restart::Done(RestartReport {
        outcome,
        health: "connected".into(),
    }))
}

#[test]
fn the_active_unit_serving_this_config_restarts_its_broker() {
    let host = host(|host| {
        host.unit_installed = our_unit();
        host.start(false);
        host.unit_active = true;
    });
    assert_eq!(
        root_restart(&host, None),
        done(RestartOutcome::ServiceRestarted)
    );
    assert_eq!(
        calls(&host),
        ["systemctl restart", "rejoin"],
        "never stopped or launched"
    );
    assert_eq!(
        host.lock().unwrap().serving.unwrap().instance,
        2,
        "a new broker"
    );
}

/// #2615 review (HIGH): the unit installed but stopped while an on-demand
/// broker holds the lock: that broker is stopped first, so the restarted
/// unit serves, and only a new broker counts as restarted.
#[test]
fn an_on_demand_broker_standing_in_for_the_unit_is_stopped_before_the_unit_starts() {
    let host = host(|host| {
        host.unit_installed = our_unit();
        host.start(true);
    });
    assert_eq!(
        root_restart(&host, None),
        done(RestartOutcome::ServiceRestarted)
    );
    assert_eq!(calls(&host), ["stop", "systemctl restart", "rejoin"]);
    let host = host.lock().unwrap();
    assert!(host.unit_active);
    assert_eq!(host.serving.map(|b| b.on_demand), Some(false));
}

/// #2615 review round 2: a unit that does not serve after its starts
/// leaves a broker started on demand serving, not none.
#[test]
fn a_unit_that_never_serves_falls_back_to_a_broker_started_on_demand() {
    let host = host(|host| {
        host.unit_installed = our_unit();
        host.unit_never_serves = true;
        host.start(true);
    });
    assert_eq!(root_restart(&host, None), done(RestartOutcome::Restarted));
    assert_eq!(
        calls(&host),
        [
            "stop",
            "systemctl restart",
            "systemctl restart",
            "launch",
            "rejoin"
        ]
    );
    assert_eq!(
        host.lock().unwrap().serving.map(|b| b.on_demand),
        Some(true)
    );
}

/// #2615 review round 2: a restart that leaves the old broker answering is
/// reported, never claimed.
#[test]
fn the_old_broker_answering_on_is_not_a_restart() {
    let host = host(|host| {
        host.unit_installed = our_unit();
        host.start(false);
        host.unit_active = true;
        host.restart_noop = true;
    });
    let error = root_restart(&host, None).unwrap_err();
    assert_eq!(
        error,
        RestartError::Restart(
            "quecto-admission-broker.service was restarted but no new broker answered at /home/me/.quecto/admission, nor did one started on demand"
                .into()
        )
    );
    assert_eq!(
        host.lock().unwrap().serving.unwrap().instance,
        1,
        "the old one"
    );
}

/// #2615 review round 2: an on-demand broker serving while the unit reads
/// active is still stopped first; the unit is not "restarted" around it.
#[test]
fn an_on_demand_broker_is_stopped_even_when_the_unit_reads_active() {
    let host = host(|host| {
        host.unit_installed = our_unit();
        host.start(true);
        host.unit_active = true;
    });
    assert_eq!(
        root_restart(&host, None),
        done(RestartOutcome::ServiceRestarted)
    );
    assert_eq!(calls(&host), ["stop", "systemctl restart", "rejoin"]);
    assert_eq!(
        host.lock().unwrap().serving.map(|b| b.on_demand),
        Some(false)
    );
}

/// #2615 review round 2: a broker that does not answer its owner is not
/// taken for "nothing running": only an active unit restarts around it;
/// otherwise the restart is refused, naming how to stop it.
#[test]
fn a_broker_that_does_not_answer_is_not_taken_for_none() {
    let refused = host(|host| {
        host.start(true);
        host.wedged = true;
    });
    let error = root_restart(&refused, None).unwrap_err();
    assert_eq!(
        error,
        RestartError::Restart(
            "the admission broker at /home/me/.quecto/admission does not answer its owner (admission broker at /home/me/.quecto/admission answered nothing within 5 s (is it stopped?)); stop it (`fuser -k -TERM /home/me/.quecto/admission/authority.lock`) and restart again"
                .into()
        )
    );
    assert!(
        calls(&refused).is_empty(),
        "nothing launched: {:?}",
        calls(&refused)
    );
    let unit = host(|host| {
        host.unit_installed = our_unit();
        host.start(false);
        host.unit_active = true;
        host.wedged = true;
    });
    assert_eq!(
        root_restart(&unit, None),
        done(RestartOutcome::ServiceRestarted)
    );
    assert_eq!(calls(&unit), ["systemctl restart", "rejoin"]);
}

#[test]
fn a_running_broker_without_a_unit_is_stopped_and_started_on_demand() {
    let host = host(|host| {
        host.start(true);
    });
    assert_eq!(root_restart(&host, None), done(RestartOutcome::Restarted));
    assert_eq!(calls(&host), ["stop", "launch", "rejoin"]);
    assert_eq!(host.lock().unwrap().serving.unwrap().instance, 2);
}

#[test]
fn a_unit_for_another_config_is_not_this_brokers_manager() {
    let host = host(|host| {
        host.unit_installed = Some(unit_for("/elsewhere/config.json"));
        host.start(false);
    });
    assert_eq!(root_restart(&host, None), done(RestartOutcome::Restarted));
    assert_eq!(calls(&host), ["stop", "launch", "rejoin"]);
}

/// #2615 review: only an exact `ExecStart=` line is this broker's unit.
#[test]
fn only_the_exact_exec_start_line_is_this_brokers_unit() {
    let config = Path::new("/home/me/.quecto/config.json");
    assert!(unit_serves_config(&our_unit().unwrap(), config));
    let extended = our_unit()
        .unwrap()
        .replace(".json\"\n", ".json\" --directory \"/x\"\n");
    assert!(!unit_serves_config(&extended, config), "{extended}");
    let unquoted = "ExecStart=/usr/bin/quecto admission-broker run --config \"/home/me/.quecto/config.json\"\n";
    assert!(!unit_serves_config(unquoted, config));
    let spaced = "ExecStart=\"/opt/my \\\"q\\\"/quecto\" admission-broker run --config \"/home/me/.quecto/config.json\"\n";
    assert!(unit_serves_config(spaced, config));
}

#[test]
fn a_broker_that_was_not_running_is_started() {
    let host = host(|_| {});
    assert_eq!(root_restart(&host, None), done(RestartOutcome::Started));
    assert_eq!(calls(&host), ["launch", "rejoin"], "nothing to stop");
}

#[test]
fn a_refused_stop_or_a_broker_that_never_stops_is_reported() {
    let refused = host(|host| {
        host.start(true);
        host.stop_refused = true;
    });
    let error = format!("{:?}", root_restart(&refused, None).unwrap_err());
    assert!(error.contains("unauthorized"), "{error}");
    assert_eq!(
        calls(&refused),
        ["stop"],
        "nothing started over a refused stop"
    );
    let stuck = host(|host| {
        host.start(true);
        host.stop_ignored = true;
    });
    let error = format!("{:?}", root_restart(&stuck, None).unwrap_err());
    assert!(error.contains("did not stop"), "{error}");
    assert_eq!(calls(&stuck), ["stop"]);
}

/// #2615 review: a broker other sessions or sub-agents share is restarted
/// only once confirmed; unconfirmed, nothing is touched.
#[test]
fn a_shared_broker_restarts_only_once_confirmed() {
    let host = host(|host| {
        host.start(true);
        host.live_scopes = 4;
    });
    assert_eq!(
        root_restart(&host, None),
        Ok(Restart::Unconfirmed { others: 3 })
    );
    assert!(
        calls(&host).is_empty(),
        "nothing touched: {:?}",
        calls(&host)
    );
    assert_eq!(
        root_restart(&host, Some(2)),
        Ok(Restart::Unconfirmed { others: 3 }),
        "acknowledged fewer than are registered now"
    );
    assert!(calls(&host).is_empty());
    assert_eq!(
        root_restart(&host, Some(3)),
        done(RestartOutcome::Restarted)
    );
}

#[test]
fn only_a_root_restarts_the_broker() {
    let host = host(|host| {
        host.start(true);
    });
    assert_eq!(
        use_case(&host, None, Ok(())).execute(Some(usize::MAX)),
        Err(RestartError::NotConfigured)
    );
    assert_eq!(
        use_case(&host, Some(JoinedAuthority::Child), Ok(())).execute(Some(usize::MAX)),
        Err(RestartError::Child)
    );
    assert!(
        calls(&host).is_empty(),
        "nothing touched: {:?}",
        calls(&host)
    );
}

#[test]
fn a_broker_back_that_the_session_cannot_rejoin_says_both() {
    let host = host(|_| {});
    let outcome = use_case(
        &host,
        Some(JoinedAuthority::Root(launch())),
        Err("restart required".into()),
    )
    .execute(None);
    assert_eq!(
        outcome,
        Err(RestartError::Rejoin {
            outcome: RestartOutcome::Started,
            reason: "restart required".into(),
        })
    );
    assert_eq!(calls(&host), ["launch", "rejoin"]);
}

/// An admin whose inspect holds a restart until the test releases it, so
/// two restarts overlap; bounded, so a restart that never inspects fails the
/// test rather than hanging it.
struct HoldingAdmin {
    entered: Mutex<std::sync::mpsc::Sender<()>>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
}

const HOLD: std::time::Duration = std::time::Duration::from_secs(10);

impl AuthorityAdmin for HoldingAdmin {
    fn inspect(&self, directory: &Path) -> Result<AuthorityReport, AuthorityAdminError> {
        let _ = self.entered.lock().unwrap().send(());
        let _ = self.release.lock().unwrap().recv_timeout(HOLD);
        Err(AuthorityAdminError::NotRunning {
            directory: directory.to_path_buf(),
            reason: "held".into(),
        })
    }
    fn reset(&self, _directory: &Path) -> Result<ResetReport, AuthorityAdminError> {
        unreachable!()
    }
    fn stop(&self, _directory: &Path) -> Result<(), AuthorityAdminError> {
        unreachable!()
    }
}

/// A launcher whose broker never answers and cannot start: a restart ends
/// right after its one inspect.
struct Unstartable;

impl AuthorityLauncher for Unstartable {
    fn probe(&self, _directory: &Path) -> Result<(), String> {
        Err("connection refused".into())
    }
    fn launch(&self, _launch: &AuthorityLaunch) -> Result<Box<dyn LaunchedAuthority>, String> {
        Err("held".into())
    }
    fn pause(&self, _attempt: u32) {}
}

#[test]
fn one_restart_runs_at_a_time() {
    let host = host(|_| {});
    let (entered_tx, entered) = std::sync::mpsc::channel();
    let (release, release_rx) = std::sync::mpsc::channel();
    let launcher = Arc::new(Unstartable);
    let use_case = Arc::new(RestartAuthority::new(
        Arc::new(HoldingAdmin {
            entered: Mutex::new(entered_tx),
            release: Mutex::new(release_rx),
        }),
        Arc::new(Manager(host.clone())),
        launcher.clone(),
        Arc::new(NegotiateAuthority::new(launcher)),
        Arc::new(Session {
            joined: Some(JoinedAuthority::Root(launch())),
            rejoin: Ok(()),
            host: host.clone(),
        }),
    ));
    let first = std::thread::spawn({
        let use_case = use_case.clone();
        move || use_case.execute(None)
    });
    entered
        .recv_timeout(HOLD)
        .expect("the first restart reached its inspect");
    assert_eq!(use_case.execute(None), Err(RestartError::InProgress));
    release.send(()).unwrap();
    let _ = first.join().unwrap();
    // Released either way: a later restart runs again.
    let again = std::thread::spawn({
        let use_case = use_case.clone();
        move || use_case.execute(None)
    });
    entered
        .recv_timeout(HOLD)
        .expect("a later restart runs again");
    release.send(()).unwrap();
    assert_ne!(again.join().unwrap(), Err(RestartError::InProgress));
}

/// #2615 review round 3: a wedged broker with its unit installed but
/// inactive is refused; the unit is never restarted around it.
#[test]
fn a_wedged_broker_with_an_inactive_unit_is_refused() {
    let host = host(|host| {
        host.unit_installed = our_unit();
        host.start(true);
        host.wedged = true;
    });
    let error = root_restart(&host, None).unwrap_err();
    assert!(
        matches!(&error, RestartError::Restart(reason) if reason.contains("fuser -k -TERM")),
        "{error:?}"
    );
    assert!(
        !calls(&host).iter().any(|call| call == "systemctl restart"),
        "{:?}",
        calls(&host)
    );
}

/// #2615 review round 3: a broker whose status fails (but answers) is not
/// restarted around, and its sharing is not skipped: nothing is touched.
#[test]
fn a_failing_status_refuses_and_touches_nothing() {
    let host = host(|host| {
        host.unit_installed = our_unit();
        host.start(false);
        host.unit_active = true;
        host.status_fails = true;
        host.live_scopes = 3;
    });
    assert_eq!(
        root_restart(&host, None),
        Err(RestartError::Restart(
            "the admission broker at /home/me/.quecto/admission could not report who shares it (protocol violation); nothing was restarted"
                .into()
        ))
    );
    assert!(calls(&host).is_empty(), "{:?}", calls(&host));
}
