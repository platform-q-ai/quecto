//! Contract for [`EnvironmentMemberShutdown`] (#1939), proven on the
//! production adapter over the registry-backed claims, the one-edge UDS
//! routing and the supervised owned-handle fallback: every member is asked
//! to shut down over its own edge exactly once, a member this session holds
//! no process for is compensated without a signal (unobserved when its
//! exit never arrives), a member already gone is reported already exited,
//! a member whose end another path owns and never settles is reported
//! unsettled, and a row that is not a delegated agent is refused rather
//! than signalled. Membership leaves the environment record with the
//! compensation and never mints a second claim on a claimed environment.
use std::sync::{Arc, Mutex};
use std::time::Duration;

use quecto::application::environments::ports::{EnvironmentMemberShutdown, MemberShutdownResult};
use quecto::application::subagents::ports::{DelegatedAgentRegistry, TerminationCause};
use quecto::application::subagents::use_cases::{SettleDelegatedChild, SettleDelegatedChildPorts};
use quecto::domain::environment_registry::{
    EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus, mint_environment_uuid,
};
use quecto::domain::ids::AgentUuid;
use quecto::domain::subagent_teardown::{DelegatedAgentIdentity, LaunchGeneration};
use quecto::infrastructure::processes::direct_child_routing::UdsDirectChildRouting;
use quecto::infrastructure::processes::owned_child_termination::SupervisedChildTermination;
use quecto::infrastructure::tools::environment_member_shutdown::DelegatedMemberShutdown;
use quecto::infrastructure::tools::subagent_registry::{
    SubagentEntry, SubagentRegistry, SubagentStatus,
};
use quecto::infrastructure::tools::subagent_teardown_registry::RegistryDelegatedAgents;

fn launched(uuid: &str, socket: &std::path::Path, generation: u64) -> SubagentEntry {
    let mut entry =
        SubagentEntry::with_identity(AgentUuid::new(uuid), uuid.into(), socket.to_path_buf(), 0);
    entry.launch_generation = Some(LaunchGeneration::new(generation));
    entry
}

/// A fake direct child answering every correlated command with `ok` and
/// recording what it received.
fn fake_child() -> (std::path::PathBuf, Arc<Mutex<Vec<serde_json::Value>>>) {
    let path = std::env::temp_dir().join(format!("q-ems-{}.sock", uuid::Uuid::new_v4().simple()));
    let _ = std::fs::remove_file(&path);
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = requests.clone();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                return;
            };
            let seen = seen.clone();
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
                seen.lock().unwrap().push(request.clone());
                let reply = serde_json::json!({
                    "type": "response", "id": request["id"], "command": request["type"],
                    "success": true,
                });
                let _ = write.write_all(format!("{reply}\n").as_bytes()).await;
                let _ = quecto_line_io::read_frame_or_legacy_line(&mut reader, 1024).await;
            });
        }
    });
    (path, requests)
}

struct Rig {
    registry: SubagentRegistry,
    agents: Arc<RegistryDelegatedAgents>,
    environments: EnvironmentRegistry,
    env_ref: String,
    port: Arc<dyn EnvironmentMemberShutdown>,
}

fn rig(rows: Vec<(&str, SubagentEntry)>) -> Rig {
    rig_with_kill(rows, vec!["true".into()])
}

fn rig_with_kill(rows: Vec<(&str, SubagentEntry)>, retained_kill_argv: Vec<String>) -> Rig {
    let registry: SubagentRegistry = Arc::new(Mutex::new(Default::default()));
    let environments = EnvironmentRegistry::new();
    let env_ref = environments.mint_ref().unwrap();
    environments.commit(EnvironmentRecord {
        environment_ref: env_ref.clone(),
        environment_id: "env-contract".into(),
        environment_uuid: mint_environment_uuid(),
        name: None,
        workspace_path: std::path::PathBuf::from("/workspace"),
        repository: String::new(),
        script_name: "default".into(),
        retained_exec_argv: vec![],
        retained_kill_argv,
        retained_cleanup_argv: vec![],
        retained_inspect_argv: vec![],
        members: vec![],
        status: EnvironmentStatus::Running,
        metadata: serde_json::json!({}),
        last_error: None,
        origin: quecto::domain::environment_registry::EnvironmentOrigin::Created,
        created_by: String::new(),
        created_at: None,
    });
    {
        let mut entries = registry.lock().unwrap();
        for (key, mut entry) in rows {
            entry.environment_registry = Some(environments.clone());
            entry.environment_ref = Some(env_ref.clone());
            environments.add_member(&env_ref, key).unwrap();
            entries.insert(key.into(), entry);
        }
    }
    let agents = Arc::new(
        RegistryDelegatedAgents::new(
            registry.clone(),
            None,
            None,
            quecto::composition::environments::build_member_finalizer,
        )
        .with_compensation_wait(Duration::from_millis(300)),
    );
    let settle = Arc::new(SettleDelegatedChild::new(SettleDelegatedChildPorts {
        registry: agents.clone(),
        routing: Arc::new(
            UdsDirectChildRouting::new(registry.clone()).with_timeout(Duration::from_millis(500)),
        ),
        termination: Arc::new(SupervisedChildTermination::new(registry.clone())),
        compensation: agents.clone(),
    }));
    let port: Arc<dyn EnvironmentMemberShutdown> =
        Arc::new(DelegatedMemberShutdown::new(agents.clone(), settle));
    Rig {
        registry,
        agents,
        environments,
        env_ref,
        port,
    }
}

#[tokio::test]
async fn members_are_asked_over_their_own_edge_and_compensated_without_a_signal() {
    let (a_socket, a_requests) = fake_child();
    let dead = std::env::temp_dir().join(format!("q-ems-dead-{}.sock", uuid::Uuid::new_v4()));
    let rig = rig(vec![
        ("A", launched("A", &a_socket, 1)),
        ("D", launched("D", &dead, 2)),
    ]);
    // The environment is claimed by the caller before members are asked.
    let claim = rig.environments.begin_kill(&rig.env_ref).unwrap();
    let report = rig
        .port
        .shutdown_members(&["A".into(), "D".into(), "Z".into()])
        .await;
    assert!(report.all_settled(), "{report:?}");
    let results: Vec<(&str, MemberShutdownResult)> = report
        .settled
        .iter()
        .map(|m| (m.member.as_str(), m.result))
        .collect();
    assert_eq!(
        results,
        [
            // Acknowledged, but this session holds no process for A and no
            // exit arrived within the bound: compensated unobserved.
            ("A", MemberShutdownResult::Unobserved),
            // Unreachable, unowned: compensated unobserved, never signalled.
            ("D", MemberShutdownResult::Unobserved),
            // Never a row: nothing left to ask.
            ("Z", MemberShutdownResult::AlreadyExited),
        ]
    );
    let requests = a_requests.lock().unwrap();
    assert_eq!(requests.len(), 1, "A was asked exactly once: {requests:?}");
    assert_eq!(requests[0]["type"], "shutdown");
    assert_eq!(requests[0]["reason"], "operator_request");
    // Both rows were compensated (their terminal effects ran) and their
    // membership left the claimed environment without a second claim.
    let entries = rig.registry.lock().unwrap();
    for key in ["A", "D"] {
        assert_eq!(entries[key].status, SubagentStatus::Exited, "{key}");
    }
    let record = rig.environments.get(&rig.env_ref).unwrap();
    assert!(record.members.is_empty(), "{record:?}");
    assert_eq!(record.status, EnvironmentStatus::Killing);
    rig.environments.complete_kill(claim);
    let _ = std::fs::remove_file(&a_socket);
}

#[tokio::test]
async fn a_member_whose_end_another_path_owns_and_never_settles_is_unsettled() {
    let (a_socket, a_requests) = fake_child();
    let rig = rig(vec![("A", launched("A", &a_socket, 1))]);
    let a = DelegatedAgentIdentity::new("A", LaunchGeneration::new(1));
    rig.agents
        .claim_stopping(&a, TerminationCause::SelectedTermination)
        .unwrap();
    let report = rig.port.shutdown_members(&["A".into()]).await;
    assert!(report.settled.is_empty(), "{report:?}");
    assert_eq!(report.unsettled.len(), 1);
    assert_eq!(report.unsettled[0].member, "A");
    assert!(
        report.unsettled[0].detail.contains("already in flight"),
        "{report:?}"
    );
    assert!(
        a_requests.lock().unwrap().is_empty(),
        "the owning path asks; this one only joins"
    );
    // The member is still recorded: a retry can ask again once the owner
    // settles.
    assert_eq!(rig.environments.get(&rig.env_ref).unwrap().members, ["A"]);
    let _ = std::fs::remove_file(&a_socket);
}

/// #1953 review (1): a member whose `agent_cmd kill` failed after effects
/// (the claim kept, its owner returned) is not a dead end for
/// `kill_container`: the member shutdown re-takes the claim, asks again,
/// compensates the acknowledged handle-less member `unobserved` once its
/// exit does not arrive within the bound, and the retained kill — the
/// box's real authority — runs exactly once; the environment is stopped.
#[tokio::test]
async fn kill_container_after_a_failed_member_kill_re_attempts_and_runs_the_retained_kill_once() {
    use quecto::application::environments::use_cases::KillEnvironment;
    use quecto::domain::environment_registry::EnvironmentTarget;
    use quecto::infrastructure::tools::environment_commands::ScriptEnvironmentCommands;
    use quecto::infrastructure::tools::subagent_registry::{ClaimOwner, TeardownPhase};

    let marker = std::env::temp_dir().join(format!("q-ems-kill-{}", uuid::Uuid::new_v4().simple()));
    let (a_socket, a_requests) = fake_child();
    let rig = rig_with_kill(
        vec![("A", launched("A", &a_socket, 1))],
        vec![
            "sh".into(),
            "-c".into(),
            format!("echo killed >> {}", marker.display()),
        ],
    );
    let a = DelegatedAgentIdentity::new("A", LaunchGeneration::new(1));
    // What a kill that failed after effects leaves behind: the claim kept
    // for the exit, its owner returned.
    rig.agents
        .claim_stopping(&a, TerminationCause::SelectedTermination)
        .unwrap();
    rig.agents.retain_stopping(&a);
    assert!(matches!(
        rig.registry.lock().unwrap()["A"].teardown_phase(),
        TeardownPhase::Stopping(claim) if claim.owner == ClaimOwner::Returned
    ));

    let kill = KillEnvironment::new(
        rig.environments.clone(),
        rig.port.clone(),
        Arc::new(ScriptEnvironmentCommands::inline()),
    );
    let killed = kill
        .kill_container(&EnvironmentTarget::Ref(rig.env_ref.clone()))
        .await
        .expect("the retained kill ran after the member settled");
    assert_eq!(killed.members.settled.len(), 1, "{killed:?}");
    assert_eq!(killed.members.settled[0].member, "A");
    assert_eq!(
        killed.members.settled[0].result,
        MemberShutdownResult::Unobserved,
        "asked again, exit not observed within the bound, compensated truthfully"
    );
    assert!(killed.members.unsettled.is_empty());
    assert_eq!(
        a_requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r["type"] == "shutdown")
            .count(),
        1,
        "the retry sent its own shutdown"
    );
    assert_eq!(
        std::fs::read_to_string(&marker)
            .unwrap_or_default()
            .lines()
            .count(),
        1,
        "the retained kill ran exactly once"
    );
    assert_eq!(
        rig.environments.get(&rig.env_ref).unwrap().status,
        EnvironmentStatus::Stopped
    );
    assert_eq!(
        rig.registry.lock().unwrap()["A"].teardown_phase(),
        TeardownPhase::Compensated
    );
    let _ = std::fs::remove_file(&marker);
    let _ = std::fs::remove_file(&a_socket);
}

#[tokio::test]
async fn a_row_that_is_not_a_delegated_agent_is_refused_not_signalled() {
    let rig = rig(vec![(
        "F",
        SubagentEntry::new(std::path::PathBuf::from("/tmp/fixture.sock"), 0),
    )]);
    let report = rig.port.shutdown_members(&["F".into()]).await;
    assert_eq!(report.unsettled.len(), 1, "{report:?}");
    assert!(
        report.unsettled[0].detail.contains("not a delegated agent"),
        "{report:?}"
    );
    assert_eq!(
        rig.registry.lock().unwrap()["F"].status,
        SubagentStatus::Starting,
        "the row is left as it was"
    );
}

#[tokio::test]
async fn port_is_object_safe_and_send() {
    let rig = rig(vec![]);
    let port = rig.port.clone();
    let report = tokio::spawn(async move { port.shutdown_members(&[]).await })
        .await
        .unwrap();
    assert!(report.all_settled());
    assert!(report.settled.is_empty());
}
