//! Contract for [`DelegatedAgentRegistry`] (#1936), proven on the production
//! registry adapter: identity resolution by uuid or live display label with
//! stable refusals, the stopping / terminal claims that make every path
//! converge exactly once, and compensation observation with a bound.
//!
//! The same file proves the lineage the kill routes over: a descendant
//! reported with its launch generation is a deeper record parented by the
//! row that reported it, so the real one-edge routing adapter forwards a
//! nested target through its direct ancestor and never sends that ancestor
//! self shutdown.
use std::sync::{Arc, Mutex};
use std::time::Duration;

use quecto::application::subagents::dto::{
    KillDelegatedAgentError, KillDelegatedAgentRequest, TerminateDelegatedAgentRequest,
    TerminationRouted,
};
use quecto::application::subagents::ports::{
    CompensationObservation, DelegatedAgentRegistry, ResolutionError, StoppingClaimError,
    SubagentLifecycleRepository, TerminalClaim, TerminationCause,
};
use quecto::application::subagents::use_cases::{
    KillDelegatedAgent, KillDelegatedAgentPorts, TerminateDelegatedAgent,
};
use quecto::composition::subagent_teardown::LifecycleAdapter as RegistryLifecycleRepository;
use quecto::domain::ids::AgentUuid;
use quecto::domain::subagent_teardown::{
    DelegatedAgentIdentity, LaunchGeneration, RoutingDepth, TerminationRouteError,
};
use quecto::infrastructure::processes::direct_child_routing::UdsDirectChildRouting;
use quecto::infrastructure::processes::owned_child_termination::SupervisedChildTermination;
use quecto::infrastructure::tools::subagent_registry::{
    SubagentEntry, SubagentRegistry, TeardownPhase,
};
use quecto::infrastructure::tools::subagent_teardown_registry::RegistryDelegatedAgents;

fn identity(uuid: &str, generation: u64) -> DelegatedAgentIdentity {
    DelegatedAgentIdentity::new(uuid, LaunchGeneration::new(generation))
}

fn launched(uuid: &str, display: &str, socket: &std::path::Path, generation: u64) -> SubagentEntry {
    let mut entry = SubagentEntry::with_identity(
        AgentUuid::new(uuid),
        display.into(),
        socket.to_path_buf(),
        0,
    );
    entry.launch_generation = Some(LaunchGeneration::new(generation));
    entry
}

fn reported(uuid: &str, display: &str, parent: &str, generation: u64) -> SubagentEntry {
    let mut entry = SubagentEntry::with_identity(
        AgentUuid::new(uuid),
        display.into(),
        std::path::PathBuf::new(),
        4242,
    );
    entry.reported_generation = Some(LaunchGeneration::new(generation));
    entry.parent_id = Some(parent.into());
    entry
}

/// A fake direct child answering every correlated command with `ok` and
/// recording what it received.
fn fake_child() -> (std::path::PathBuf, Arc<Mutex<Vec<serde_json::Value>>>) {
    let path = std::env::temp_dir().join(format!("q-dar-{}.sock", uuid::Uuid::new_v4().simple()));
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

/// root → A → B, A → C, root → D; A and D launched here, B and C reported.
fn tree(a_socket: &std::path::Path, d_socket: &std::path::Path) -> SubagentRegistry {
    let registry: SubagentRegistry = Arc::new(Mutex::new(Default::default()));
    {
        let mut entries = registry.lock().unwrap();
        entries.insert("A".into(), launched("A", "alpha", a_socket, 1));
        entries.insert("B".into(), reported("B", "bravo", "A", 1));
        entries.insert("C".into(), reported("C", "charlie", "A", 1));
        entries.insert("D".into(), launched("D", "delta", d_socket, 2));
    }
    registry
}

fn port(registry: &SubagentRegistry) -> Arc<dyn DelegatedAgentRegistry> {
    Arc::new(
        RegistryDelegatedAgents::new(registry.clone(), None, None)
            .with_compensation_wait(Duration::from_millis(200)),
    )
}

#[tokio::test]
async fn resolution_is_stable_for_uuids_labels_ambiguity_and_dead_rows() {
    let (a, _) = fake_child();
    let (d, _) = fake_child();
    let registry = tree(&a, &d);
    let port = port(&registry);
    assert_eq!(port.resolve("A"), Ok(identity("A", 1)));
    assert_eq!(port.resolve("delta"), Ok(identity("D", 2)));
    assert_eq!(
        port.resolve("B"),
        Ok(identity("B", 1)),
        "a reported row is addressable"
    );
    assert_eq!(port.resolve("ghost"), Err(ResolutionError::Unknown));
    registry
        .lock()
        .unwrap()
        .insert("X".into(), launched("X", "alpha", &a, 7));
    assert_eq!(port.resolve("alpha"), Err(ResolutionError::Ambiguous));
    registry.lock().unwrap()["D"]
        .teardown
        .send_replace(TeardownPhase::Compensated);
    assert_eq!(port.resolve("D"), Err(ResolutionError::Exited));
    registry.lock().unwrap().insert(
        "fixture".into(),
        SubagentEntry::new(std::path::PathBuf::from("/tmp/f.sock"), 0),
    );
    assert_eq!(port.resolve("fixture"), Err(ResolutionError::NotDelegated));
}

#[tokio::test]
async fn claims_are_exclusive_and_observation_is_bounded() {
    let (a, _) = fake_child();
    let (d, _) = fake_child();
    let registry = tree(&a, &d);
    let port = port(&registry);
    let a_id = identity("A", 1);
    assert_eq!(
        port.claim_stopping(&identity("A", 2), TerminationCause::SelectedTermination),
        Err(StoppingClaimError::Unknown)
    );
    assert_eq!(
        port.claim_stopping(&a_id, TerminationCause::SelectedTermination),
        Ok(())
    );
    assert_eq!(
        port.claim_stopping(&a_id, TerminationCause::SelectedTermination),
        Err(StoppingClaimError::AlreadyStopping)
    );
    assert!(!port.terminal_claimed(&a_id), "stopping is not terminal");
    assert_eq!(port.claim_terminal(&a_id), TerminalClaim::Claimed);
    assert!(port.terminal_claimed(&a_id));
    assert_eq!(port.claim_terminal(&a_id), TerminalClaim::AlreadyClaimed);
    assert!(!port.terminal_claimed(&identity("ghost", 1)));
    assert_eq!(
        port.await_compensated(&a_id).await,
        CompensationObservation::TimedOut
    );
    let waiter = {
        let port = port.clone();
        tokio::spawn(async move { port.await_compensated(&identity("A", 1)).await })
    };
    tokio::time::sleep(Duration::from_millis(20)).await;
    registry.lock().unwrap()["A"]
        .teardown
        .send_replace(TeardownPhase::Compensated);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(2), waiter)
            .await
            .unwrap()
            .unwrap(),
        CompensationObservation::Compensated
    );
    assert_eq!(
        port.await_compensated(&identity("ghost", 1)).await,
        CompensationObservation::Unknown
    );
    assert!(!port.holds_process(&a_id));
}

/// The production lineage adapter lists reported descendants beneath the
/// row that reported them, and the real routing adapter resolves one edge:
/// a nested target is forwarded through A, A itself gets self shutdown.
#[tokio::test]
async fn nested_targets_are_forwarded_through_their_direct_ancestor_only() {
    let (a, a_requests) = fake_child();
    let (d, d_requests) = fake_child();
    let registry = tree(&a, &d);
    let lifecycle = Arc::new(RegistryLifecycleRepository::new(
        Some(registry.clone()),
        AgentUuid::new("root"),
    ));
    let lineage = lifecycle.lineage();
    assert_eq!(lineage.records.len(), 4);
    assert!(
        lineage
            .records
            .iter()
            .any(|r| r.identity == identity("B", 1) && r.parent == AgentUuid::new("A"))
    );
    let direct: Vec<_> = lineage.direct_children().cloned().collect();
    assert_eq!(direct, [identity("A", 1), identity("D", 2)]);
    let route = TerminateDelegatedAgent::new(
        lifecycle,
        Arc::new(UdsDirectChildRouting::new(registry.clone())),
    );
    let routed = route
        .execute(TerminateDelegatedAgentRequest {
            target: identity("B", 1),
            remaining_depth: RoutingDepth::new(RoutingDepth::MAX_HOPS).unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(
        routed,
        TerminationRouted::Forwarded {
            via: identity("A", 1),
            remaining_depth: RoutingDepth::new(RoutingDepth::MAX_HOPS - 1).unwrap(),
        }
    );
    let seen = a_requests.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0]["type"], "terminate_delegated_agent");
    assert_eq!(seen[0]["target_uuid"], "B");
    assert_eq!(seen[0]["target_generation"], 1);
    assert!(d_requests.lock().unwrap().is_empty(), "D is untouched");
    // Stale generation, a dead target and a target this harness never saw
    // are refused before any edge.
    for (target, expected) in [
        (
            identity("B", 9),
            TerminationRouteError::StaleGeneration {
                target: AgentUuid::new("B"),
                requested: LaunchGeneration::new(9),
                current: LaunchGeneration::new(1),
            },
        ),
        (
            identity("ghost", 1),
            TerminationRouteError::UnknownTarget(AgentUuid::new("ghost")),
        ),
    ] {
        let error = route
            .execute(TerminateDelegatedAgentRequest {
                target,
                remaining_depth: RoutingDepth::new(4).unwrap(),
            })
            .await
            .unwrap_err();
        assert_eq!(
            error,
            quecto::application::subagents::dto::TerminateDelegatedAgentError::Rejected(expected)
        );
    }
    assert_eq!(a_requests.lock().unwrap().len(), 1, "no further edge");
}

/// The kill use case over the production adapters: killing nested B waits
/// for the row's compensation (driven by A's reported snapshot in
/// production), never touches A's own lifecycle, and reports B's subtree.
#[tokio::test]
async fn killing_a_nested_target_leaves_its_ancestor_and_siblings_live() {
    let (a, a_requests) = fake_child();
    let (d, _) = fake_child();
    let registry = tree(&a, &d);
    let agents = Arc::new(
        RegistryDelegatedAgents::new(registry.clone(), None, None)
            .with_compensation_wait(Duration::from_secs(5)),
    );
    let lifecycle = Arc::new(RegistryLifecycleRepository::new(
        Some(registry.clone()),
        AgentUuid::new("root"),
    ));
    let kill = KillDelegatedAgent::new(
        Arc::new(TerminateDelegatedAgent::new(
            lifecycle.clone(),
            Arc::new(UdsDirectChildRouting::new(registry.clone())),
        )),
        KillDelegatedAgentPorts {
            registry: agents.clone(),
            lifecycle,
            termination: Arc::new(SupervisedChildTermination::new(registry.clone())),
            compensation: agents,
        },
    );
    // A's next reported snapshot no longer lists B: the merge prune marks
    // the row dead, which is what the kill observes.
    let pruner = {
        let registry = registry.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let mut entries = registry.lock().unwrap();
            let next =
                quecto::infrastructure::tools::subagent_cascade::next_roster_sequence(&entries);
            quecto::infrastructure::tools::subagent_cascade::mark_entry_dead(
                entries.get_mut("B").unwrap(),
                next,
            );
        })
    };
    let outcome = tokio::time::timeout(
        Duration::from_secs(5),
        kill.execute(KillDelegatedAgentRequest {
            reference: "bravo".into(),
        }),
    )
    .await
    .expect("bounded")
    .unwrap();
    pruner.await.unwrap();
    assert_eq!(outcome.target, identity("B", 1));
    assert_eq!(outcome.removed, [AgentUuid::new("B")]);
    let seen = a_requests.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0]["type"], "terminate_delegated_agent",
        "never self shutdown for A"
    );
    {
        let entries = registry.lock().unwrap();
        assert_eq!(entries["A"].teardown_phase(), TeardownPhase::Live);
        assert_eq!(entries["C"].teardown_phase(), TeardownPhase::Live);
        assert_eq!(entries["D"].teardown_phase(), TeardownPhase::Live);
    }
    // A second kill of the same target has no effect: it is exited.
    let again = kill
        .execute(KillDelegatedAgentRequest {
            reference: "B".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(
        again,
        KillDelegatedAgentError::Unresolved(ResolutionError::Exited)
    );
}
