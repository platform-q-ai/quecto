//! `restart_admission` (#2615) over real in-process brokers: a root's
//! restart stops its broker, starts another on the same directory and
//! rejoins it; a child or a session without admission is told why not.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use super::*;
use crate::application::admission::ports::{
    AuthorityLaunch, AuthorityLauncher, AuthorityServiceManager, LaunchState, LaunchedAuthority,
    ServiceUnitSpec, UnitStatus,
};
use crate::application::admission::use_cases::NegotiateAuthority;
use crate::composition::admission::start_authority;
use crate::domain::admission::value_objects::inference_admission::{
    AdmissionConfig, GroupId, GroupPolicy,
};
use crate::infrastructure::admission::process::install_in;
use crate::infrastructure::admission::{
    AuthorityDirectory, CurrentProcessSession, DetachedAuthorityLauncher, Negotiation,
    ProcessAdmission, SocketAuthorityAdmin, write_admission_context,
};
use crate::infrastructure::providers::routing::provider_runtime_admission::AdmissionRuntimeProposal;

fn proposal() -> AdmissionRuntimeProposal {
    let group = GroupId::new("g").unwrap();
    AdmissionRuntimeProposal {
        policy: AdmissionConfig {
            groups: BTreeMap::from([(
                group.clone(),
                GroupPolicy {
                    capacity: 1,
                    reserve: 0,
                    min_interval_ms: 1,
                    queue_capacity: 8,
                    queue_timeout_ms: 1_000,
                    attempt_timeout_ms: 2_000,
                    fallback_base_ms: 50,
                    max_cooldown_ms: 500,
                },
            )]),
            aliases: BTreeMap::from([("acct".into(), group)]),
            max_scopes: 16,
            terminal_capacity: 32,
        },
        bindings: BTreeMap::from([("fake".into(), "acct".into())]),
    }
}

/// Serve `directory` in-process until asked to stop, then shut down as the
/// broker's run path does.
fn serve(runtime: &tokio::runtime::Runtime, directory: &Path) {
    let server = runtime
        .block_on(start_authority(
            AuthorityDirectory::open(directory).unwrap(),
            proposal(),
        ))
        .unwrap();
    runtime.spawn(async move {
        server.stopped().await;
        server.shutdown().await;
    });
}

/// Starts an in-process broker where production starts a process; probes
/// the real client socket.
struct InProcess {
    runtime: tokio::runtime::Runtime,
    probe: DetachedAuthorityLauncher,
    launches: Mutex<u32>,
}

struct Serving;

impl LaunchedAuthority for Serving {
    fn state(&mut self) -> LaunchState {
        LaunchState::Running
    }
}

impl AuthorityLauncher for InProcess {
    fn probe(&self, directory: &Path) -> Result<(), String> {
        self.probe.probe(directory)
    }
    fn launch(&self, launch: &AuthorityLaunch) -> Result<Box<dyn LaunchedAuthority>, String> {
        *self.launches.lock().unwrap() += 1;
        serve(&self.runtime, &launch.directory);
        Ok(Box::new(Serving))
    }
    fn pause(&self, _attempt: u32) {
        std::thread::sleep(Duration::from_millis(20));
    }
}

struct NoUnit;

impl AuthorityServiceManager for NoUnit {
    fn unit_path(&self) -> PathBuf {
        PathBuf::from("/nonexistent/quecto-admission-broker.service")
    }
    fn unit_name(&self) -> String {
        "quecto-admission-broker.service".into()
    }
    fn unit_status(&self) -> Result<UnitStatus, String> {
        Ok(UnitStatus::Absent)
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
        unreachable!("no unit serves this broker")
    }
    fn is_active(&self) -> Result<bool, String> {
        Ok(false)
    }
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

/// A process binding slot of the test's own.
fn slot() -> &'static OnceLock<Arc<ProcessAdmission>> {
    Box::leak(Box::new(OnceLock::new()))
}

fn restart_over(
    launcher: &Arc<InProcess>,
    binding: &'static OnceLock<Arc<ProcessAdmission>>,
) -> RestartAuthority {
    RestartAuthority::new(
        Arc::new(SocketAuthorityAdmin::new()),
        Arc::new(NoUnit),
        launcher.clone(),
        Arc::new(NegotiateAuthority::new(launcher.clone())),
        Arc::new(CurrentProcessSession::over(binding)),
    )
}

fn in_process() -> Arc<InProcess> {
    Arc::new(InProcess {
        runtime: runtime(),
        probe: DetachedAuthorityLauncher::new(PathBuf::from("/nonexistent/quecto")),
        launches: Mutex::new(0),
    })
}

#[test]
fn a_root_restarts_its_broker_and_rejoins_it() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("authority");
    let launcher = in_process();
    serve(&launcher.runtime, &directory);
    let binding = slot();
    let root = install_in(
        binding,
        Negotiation::Root {
            directory: directory.clone(),
        },
    )
    .unwrap();
    root.remember_launch(AuthorityLaunch {
        directory: directory.clone(),
        config: temp.path().join("config.json"),
        base_dir: temp.path().to_path_buf(),
    });
    assert!(root.connected());
    let data = respond(Some(&restart_over(&launcher, binding)), false).unwrap();
    assert_eq!(
        data,
        serde_json::json!({"outcome": "restarted", "authorityStatus": "connected"})
    );
    assert_eq!(*launcher.launches.lock().unwrap(), 1, "one broker started");
    assert!(root.connected(), "rejoined the new broker");
}

#[test]
fn a_session_without_admission_or_a_child_cannot_restart_the_broker() {
    assert_eq!(
        respond(None, false),
        Err("admission is not configured for this session".into())
    );
    let launcher = in_process();
    assert_eq!(
        respond(Some(&restart_over(&launcher, slot())), false),
        Err("admission is not configured for this session".into())
    );
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("authority");
    serve(&launcher.runtime, &directory);
    let root = install_in(
        slot(),
        Negotiation::Root {
            directory: directory.clone(),
        },
    )
    .unwrap();
    let credential = launcher.runtime.block_on(root.register_child()).unwrap();
    let context = temp.path().join("child.ctx");
    write_admission_context(&context, &root.endpoint(), &credential).unwrap();
    let child = slot();
    install_in(child, Negotiation::Child { context }).unwrap();
    assert_eq!(
        respond(Some(&restart_over(&launcher, child)), false),
        Err(
            "a child session cannot restart the shared admission broker; its root session can"
                .into()
        )
    );
    assert_eq!(*launcher.launches.lock().unwrap(), 0, "nothing started");
}

/// #2615 review: a broker another scope shares (here this root's own
/// sub-agent) is restarted only once confirmed.
#[test]
fn a_shared_broker_is_restarted_only_once_confirmed() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("authority");
    let launcher = in_process();
    serve(&launcher.runtime, &directory);
    let binding = slot();
    let root = install_in(
        binding,
        Negotiation::Root {
            directory: directory.clone(),
        },
    )
    .unwrap();
    root.remember_launch(AuthorityLaunch {
        directory: directory.clone(),
        config: temp.path().join("config.json"),
        base_dir: temp.path().to_path_buf(),
    });
    let _child = launcher.runtime.block_on(root.register_child()).unwrap();
    let restart = restart_over(&launcher, binding);
    assert_eq!(
        respond(Some(&restart), false),
        Ok(serde_json::json!({"outcome": "unconfirmed", "sharedWith": 1}))
    );
    assert_eq!(*launcher.launches.lock().unwrap(), 0, "nothing restarted");
    assert_eq!(
        respond(Some(&restart), true),
        Ok(serde_json::json!({"outcome": "restarted", "authorityStatus": "connected"}))
    );
    assert_eq!(*launcher.launches.lock().unwrap(), 1);
}
