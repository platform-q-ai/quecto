//! Delegated swarm member termination (#1939): protocol over the member's
//! endpoint, the delegated-agent graph for a member this harness launched,
//! and a truthful failure — never a signal — for everything else.
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::{DelegatedSwarmMemberTermination, shutdown_member_over_endpoint};
use crate::application::subagents::use_cases::{
    KillDelegatedAgent, KillDelegatedAgentPorts, OwnerConclusionPorts, TerminateDelegatedAgent,
};
use crate::domain::ids::AgentUuid;
use crate::domain::subagent_teardown::LaunchGeneration;
use crate::domain::swarm::{Member, MemberStatus, ProcessIdentity};
use crate::infrastructure::processes::direct_child_routing::UdsDirectChildRouting;
use crate::infrastructure::processes::owned_child_supervisor::{
    OwnedChildSupervisor, ProcessGroup, TerminationBudget,
};
use crate::infrastructure::processes::owned_child_termination::SupervisedChildTermination;
use crate::infrastructure::tools::harness_lifecycle::new_shared_harness_lifecycle;
use crate::infrastructure::tools::subagent_registry::{
    SubagentEntry, SubagentRegistry, SubagentStatus,
};
use crate::infrastructure::tools::subagent_teardown_registry::RegistryDelegatedAgents;
use crate::interface::cli::uds_teardown_adapters::RegistryLifecycleRepository;

/// A fake harness endpoint that acknowledges every command and records it.
fn acking_endpoint(dir: &std::path::Path) -> (std::path::PathBuf, Arc<Mutex<Vec<String>>>) {
    let path = dir.join(format!("m-{}.sock", uuid::Uuid::new_v4().simple()));
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let log = log.clone();
            tokio::spawn(async move {
                use tokio::io::AsyncWriteExt;
                let (read, mut write) = tokio::io::split(stream);
                let mut reader = tokio::io::BufReader::new(read);
                let Ok(Some(incoming)) =
                    quecto_line_io::read_frame_or_legacy_line(&mut reader, 64 * 1024).await
                else {
                    return;
                };
                let bytes = match incoming {
                    quecto_line_io::Incoming::Frame(b)
                    | quecto_line_io::Incoming::LegacyLine(b) => b,
                };
                let request: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                log.lock()
                    .unwrap()
                    .push(request["type"].as_str().unwrap_or_default().to_owned());
                let reply = serde_json::json!({
                    "type": "response", "id": request["id"], "command": request["type"],
                    "success": true,
                });
                let _ = write.write_all(format!("{reply}\n").as_bytes()).await;
                let _ = quecto_line_io::read_frame_or_legacy_line(&mut reader, 1024).await;
            });
        }
    });
    (path, seen)
}

const FAST: TerminationBudget = TerminationBudget {
    exit_after_ack: Duration::from_millis(300),
    term_grace: Duration::from_millis(300),
    kill_grace: Duration::from_secs(2),
};

fn member(id: &str, endpoint: Option<&std::path::Path>, pid: Option<u32>) -> Member {
    Member {
        id: id.into(),
        status: MemberStatus::Live,
        process: pid.map(|pid| ProcessIdentity {
            pid,
            started: "identity".into(),
        }),
        endpoint: endpoint.map(|p| p.to_string_lossy().into_owned()),
        launcher: None,
    }
}

fn graph(registry: &SubagentRegistry) -> DelegatedSwarmMemberTermination {
    let agents = Arc::new(
        RegistryDelegatedAgents::new(
            registry.clone(),
            None,
            None,
            crate::composition::environments::build_member_finalizer,
        )
        .with_compensation_wait(Duration::from_millis(300)),
    );
    let lifecycle = Arc::new(RegistryLifecycleRepository::new(
        Some(registry.clone()),
        AgentUuid::new("coordinator"),
        new_shared_harness_lifecycle(),
    ));
    let routing = Arc::new(
        UdsDirectChildRouting::new(registry.clone()).with_timeout(Duration::from_millis(500)),
    );
    let route = Arc::new(
        TerminateDelegatedAgent::new(lifecycle.clone(), routing).with_owner_conclusion(
            OwnerConclusionPorts {
                registry: agents.clone(),
                termination: Arc::new(
                    SupervisedChildTermination::new(registry.clone()).with_budgets(FAST, FAST),
                ),
                compensation: agents.clone(),
            },
        ),
    );
    let kill = Arc::new(KillDelegatedAgent::new(
        route,
        KillDelegatedAgentPorts {
            registry: agents.clone(),
            lifecycle,
            compensation: agents,
        },
    ));
    DelegatedSwarmMemberTermination::new(registry.clone(), kill)
}

#[tokio::test]
async fn a_member_without_an_endpoint_is_reported_not_signalled() {
    let error = shutdown_member_over_endpoint(&member("w", None, Some(std::process::id())))
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("registered no endpoint"),
        "{error}"
    );
    let error = shutdown_member_over_endpoint(&Member {
        endpoint: Some(String::new()),
        ..member("w", None, None)
    })
    .await
    .unwrap_err();
    assert!(
        error.to_string().contains("registered no endpoint"),
        "{error}"
    );
}

#[tokio::test]
async fn a_member_reachable_only_over_its_endpoint_is_asked_to_shut_down() {
    let dir = tempfile::tempdir().unwrap();
    let (socket, seen) = acking_endpoint(dir.path());
    shutdown_member_over_endpoint(&member("w", Some(&socket), Some(1)))
        .await
        .unwrap();
    assert_eq!(*seen.lock().unwrap(), ["shutdown"]);
    let gone = dir.path().join("gone.sock");
    let error = shutdown_member_over_endpoint(&member("w", Some(&gone), Some(1)))
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("did not accept shutdown"),
        "{error}"
    );
}

/// A member whose endpoint is the socket of a row this harness launched is
/// ended through the delegated-agent graph: asked over its edge exactly
/// once, the process identity never consulted. A row this harness holds no
/// process for has no fallback: an acknowledgement whose exit is never
/// observed is reported as a failure, truthfully, with the row left live
/// for its own monitor to end.
#[tokio::test]
async fn a_member_this_harness_launched_is_asked_through_its_delegated_agent() {
    let dir = tempfile::tempdir().unwrap();
    let (socket, seen) = acking_endpoint(dir.path());
    let registry: SubagentRegistry = Arc::new(Mutex::new(Default::default()));
    let mut entry = SubagentEntry::with_identity(
        AgentUuid::new("worker-uuid"),
        "worker".into(),
        socket.clone(),
        0,
    );
    entry.launch_generation = Some(LaunchGeneration::new(1));
    registry.lock().unwrap().insert("worker-uuid".into(), entry);
    let termination = graph(&registry);
    // The store's member id differs from the registry uuid; the endpoint
    // is the correlation.
    let error = termination
        .terminate(&member("store-member-7", Some(&socket), Some(424242)))
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("acknowledged but its exit was not observed"),
        "{error}"
    );
    assert_eq!(*seen.lock().unwrap(), ["shutdown"]);
    assert_ne!(
        registry.lock().unwrap()["worker-uuid"].status,
        SubagentStatus::Exited,
        "the row is left live for its monitor; nothing was pretended"
    );
    // Once its own path ended it, the member is found gone: no error.
    super::super::subagent_monitor::mark_exited(
        registry.lock().unwrap().get_mut("worker-uuid").unwrap(),
    );
    termination
        .terminate(&member("store-member-7", Some(&socket), Some(424242)))
        .await
        .unwrap();
    assert_eq!(*seen.lock().unwrap(), ["shutdown"], "asked once");
}

/// A row this harness owns a live process for is concluded through the
/// supervisor (protocol first, fallback after a negative outcome); a
/// process the registry does not own is untouched even when the member's
/// process identity names it.
#[tokio::test]
async fn an_owned_process_is_concluded_by_the_supervisor_and_an_unowned_one_never_signalled() {
    let dir = tempfile::tempdir().unwrap();
    let supervisor = Arc::new(OwnedChildSupervisor::new());
    let mut command = tokio::process::Command::new("sleep");
    command
        .arg("30")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let spawned = supervisor
        .spawn(command, ProcessGroup::Inherited)
        .await
        .unwrap();
    let owned_pid = spawned.display_pid.0;
    // The owned child never listens: the protocol is negative and the
    // supervisor's fallback ends it.
    let never_bound = dir.path().join("never.sock");
    let registry: SubagentRegistry = Arc::new(Mutex::new(Default::default()));
    let mut entry = SubagentEntry::with_identity(
        AgentUuid::new("owned-uuid"),
        "owned".into(),
        never_bound.clone(),
        owned_pid,
    );
    entry.launch_generation = Some(LaunchGeneration::new(1));
    entry.owned_child = Some(spawned.handle);
    entry.owned_child_supervisor = Some(supervisor.clone());
    registry.lock().unwrap().insert("owned-uuid".into(), entry);
    let termination = graph(&registry);
    termination
        .terminate(&member("owned-member", Some(&never_bound), Some(owned_pid)))
        .await
        .unwrap();
    assert_eq!(
        registry.lock().unwrap()["owned-uuid"].status,
        SubagentStatus::Exited
    );
    assert!(
        !std::path::Path::new(&format!("/proc/{owned_pid}")).exists()
            || std::fs::read_to_string(format!("/proc/{owned_pid}/stat"))
                .map(|s| s.contains(") Z"))
                .unwrap_or(true),
        "the owned sleeper was ended by its supervisor"
    );

    // A stranger's process with the member's identity: reported, untouched.
    let mut stranger = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let error = termination
        .terminate(&member(
            "stranger",
            Some(&dir.path().join("nobody.sock")),
            Some(stranger.id()),
        ))
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("not owned by this harness"),
        "{error}"
    );
    // A signal takes time to land: watch the stranger over a bounded
    // window (a reintroduced pid signal ends a `sleep` well within it).
    let watched_until = std::time::Instant::now() + Duration::from_millis(300);
    while std::time::Instant::now() < watched_until {
        assert!(
            stranger.try_wait().unwrap().is_none(),
            "the stranger's pid was signalled"
        );
        let stat = std::fs::read_to_string(format!("/proc/{}/stat", stranger.id()))
            .expect("the stranger is still a process");
        assert!(!stat.contains(") Z"), "the stranger was ended: {stat}");
        std::thread::sleep(Duration::from_millis(10));
    }
    stranger.kill().unwrap();
    stranger.wait().unwrap();
}
