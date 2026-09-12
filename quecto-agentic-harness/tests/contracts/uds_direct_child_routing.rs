//! Contract for the production [`DirectChildRouting`] adapter (#1935):
//! `UdsDirectChildRouting` resolves an identity through this harness's own
//! registry (uuid *and* launch generation of a child it launched), sends one
//! correlated command over the child's endpoint, and reports the child's
//! answer in the port's vocabulary. Merged, restored and fixture rows are
//! never direct children, so no edge is ever touched for them.
use std::sync::{Arc, Mutex};
use std::time::Duration;

use quecto::application::subagents::dto::{
    TerminateDelegatedAgentError, TerminateDelegatedAgentRequest, TerminationRouted,
};
use quecto::application::subagents::ports::{ChildRoutingError, DirectChildRouting};
use quecto::application::subagents::use_cases::TerminateDelegatedAgent;
use quecto::domain::ids::AgentUuid;
use quecto::domain::subagent_teardown::{
    DelegatedAgentIdentity, LaunchGeneration, LineageSnapshot, RoutingDepth, ShutdownReason,
    TerminationRouteError,
};
use quecto::infrastructure::processes::direct_child_routing::{
    PROTOCOL_ACK_TIMEOUT, UdsDirectChildRouting, shutdown_over_socket,
};
use quecto::infrastructure::tools::subagent_registry::{SubagentEntry, SubagentRegistry};

use super::teardown_fixture::{Lifecycle, identity, record};

/// A fake child endpoint answering every correlated command with `ok`.
fn fake_child(ok: bool) -> (std::path::PathBuf, Arc<Mutex<Vec<serde_json::Value>>>) {
    let path = std::env::temp_dir().join(format!("q-ct-{}.sock", uuid::Uuid::new_v4().simple()));
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
                    "type": "response",
                    "id": request["id"],
                    "command": request["type"],
                    "success": ok,
                    "error": if ok { serde_json::Value::Null } else { "harness already terminated".into() },
                });
                let _ = write.write_all(format!("{reply}\n").as_bytes()).await;
                let _ = quecto_line_io::read_frame_or_legacy_line(&mut reader, 1024).await;
            });
        }
    });
    (path, requests)
}

fn launched(uuid: &str, socket: &std::path::Path, generation: u64) -> SubagentEntry {
    let mut entry = SubagentEntry::with_identity(
        AgentUuid::new(uuid),
        uuid.to_owned(),
        socket.to_path_buf(),
        0,
    );
    entry.launch_generation = Some(LaunchGeneration::new(generation));
    entry
}

fn registry(entries: Vec<(&str, SubagentEntry)>) -> SubagentRegistry {
    let registry: SubagentRegistry = Arc::new(Mutex::new(Default::default()));
    for (key, entry) in entries {
        registry.lock().unwrap().insert(key.to_owned(), entry);
    }
    registry
}

#[tokio::test]
async fn adapter_is_a_direct_child_routing_port_over_a_real_endpoint() {
    let (socket, requests) = fake_child(true);
    let port: Arc<dyn DirectChildRouting> = Arc::new(UdsDirectChildRouting::new(registry(vec![(
        "A",
        launched("A", &socket, 7),
    )])));
    port.shutdown_child(&identity("A", 7), ShutdownReason::ParentShutdown)
        .await
        .unwrap();
    port.forward_termination(
        &identity("A", 7),
        &identity("B", 2),
        RoutingDepth::new(3).unwrap(),
    )
    .await
    .unwrap();
    let seen = requests.lock().unwrap().clone();
    assert_eq!(seen.len(), 2, "one correlated request per edge");
    assert_eq!(seen[0]["type"], "shutdown");
    assert_eq!(seen[0]["reason"], "parent_shutdown");
    assert_eq!(seen[1]["type"], "terminate_delegated_agent");
    assert_eq!(seen[1]["target_uuid"], "B");
    assert_eq!(seen[1]["target_generation"], 2);
    assert_eq!(seen[1]["remaining_depth"], 3);
    assert!(seen.iter().all(|r| r["id"].is_string()));
}

#[tokio::test]
async fn only_this_harnesss_launched_children_are_direct() {
    let (socket, requests) = fake_child(true);
    let mut merged = SubagentEntry::with_identity(
        AgentUuid::new("grandchild"),
        "grandchild".into(),
        socket.clone(),
        4242,
    );
    merged.parent_id = Some("A".into());
    let restored = SubagentEntry::new(socket.clone(), 4343);
    let port = UdsDirectChildRouting::new(registry(vec![
        ("A", launched("A", &socket, 2)),
        ("grandchild", merged),
        ("restored", restored),
    ]));
    for (uuid, generation, why) in [
        ("A", 1, "stale generation"),
        ("A", 3, "future generation"),
        ("grandchild", 1, "merged descendant"),
        ("restored", 1, "restored row"),
        ("ghost", 2, "unknown uuid"),
    ] {
        assert_eq!(
            port.shutdown_child(&identity(uuid, generation), ShutdownReason::ParentShutdown)
                .await,
            Err(ChildRoutingError::NotADirectChild),
            "{why}"
        );
    }
    assert!(requests.lock().unwrap().is_empty(), "no edge was touched");
}

#[tokio::test]
async fn negative_answers_and_unreachable_endpoints_are_reported_in_port_vocabulary() {
    let (refusing, _) = fake_child(false);
    let port = UdsDirectChildRouting::new(registry(vec![("A", launched("A", &refusing, 1))]))
        .with_timeout(Duration::from_secs(2));
    assert_eq!(
        port.shutdown_child(&identity("A", 1), ShutdownReason::SelectedTermination)
            .await,
        Err(ChildRoutingError::Unreachable(
            "harness already terminated".into()
        ))
    );
    let missing = std::env::temp_dir().join("q-ct-missing.sock");
    let outcome = shutdown_over_socket(
        &missing,
        ShutdownReason::ParentShutdown,
        Duration::from_secs(1),
    )
    .await;
    assert!(matches!(outcome, Err(ChildRoutingError::Unreachable(_))));
    assert!(PROTOCOL_ACK_TIMEOUT >= Duration::from_secs(1));
}

/// The selected-routing use case consumes the production adapter: targeting
/// a direct child sends it `shutdown`, targeting a grandchild forwards one
/// hop through its owner, and refused routes touch no endpoint.
#[tokio::test]
async fn selected_termination_routes_through_the_real_adapter() {
    let (a_socket, a_requests) = fake_child(true);
    let (d_socket, d_requests) = fake_child(true);
    let lineage = LineageSnapshot {
        owner: AgentUuid::new("root"),
        records: vec![
            record("A", 1, "root"),
            record("B", 1, "A"),
            record("D", 1, "root"),
        ],
    };
    let routing = Arc::new(UdsDirectChildRouting::new(registry(vec![
        ("A", launched("A", &a_socket, 1)),
        ("D", launched("D", &d_socket, 1)),
    ])));
    let use_case = TerminateDelegatedAgent::new(Lifecycle::new(lineage), routing);
    let routed = use_case
        .execute(TerminateDelegatedAgentRequest {
            target: identity("D", 1),
            remaining_depth: RoutingDepth::new(1).unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(
        routed,
        TerminationRouted::ShutdownRequested {
            child: identity("D", 1)
        }
    );
    let routed = use_case
        .execute(TerminateDelegatedAgentRequest {
            target: identity("B", 1),
            remaining_depth: RoutingDepth::new(2).unwrap(),
        })
        .await
        .unwrap();
    assert_eq!(
        routed,
        TerminationRouted::Forwarded {
            via: identity("A", 1),
            remaining_depth: RoutingDepth::new(1).unwrap(),
        }
    );
    let refused = use_case
        .execute(TerminateDelegatedAgentRequest {
            target: identity("D", 9),
            remaining_depth: RoutingDepth::new(1).unwrap(),
        })
        .await;
    assert_eq!(
        refused,
        Err(TerminateDelegatedAgentError::Rejected(
            TerminationRouteError::StaleGeneration {
                target: AgentUuid::new("D"),
                requested: LaunchGeneration::new(9),
                current: LaunchGeneration::new(1),
            }
        ))
    );
    let a = a_requests.lock().unwrap().clone();
    let d = d_requests.lock().unwrap().clone();
    assert_eq!(d.len(), 1);
    assert_eq!(d[0]["type"], "shutdown");
    assert_eq!(d[0]["reason"], "selected_termination");
    assert_eq!(a.len(), 1);
    assert_eq!(a[0]["type"], "terminate_delegated_agent");
    assert_eq!(a[0]["remaining_depth"], 1);
    let _ = DelegatedAgentIdentity::new("unused", LaunchGeneration::new(0));
}
