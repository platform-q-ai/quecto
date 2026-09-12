use std::sync::{Arc, Mutex};

use super::*;
use crate::domain::ids::AgentUuid;
use crate::domain::subagent_teardown::LaunchGeneration;
use crate::infrastructure::tools::subagent_registry::SubagentEntry;

/// A fake child endpoint: records each request and answers with a
/// correlated response whose `success` is `ok`.
fn fake_child(ok: bool) -> (std::path::PathBuf, Arc<Mutex<Vec<serde_json::Value>>>) {
    let dir = std::env::temp_dir();
    let path = dir.join(format!("q-dcr-{}.sock", uuid::Uuid::new_v4().simple()));
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
                // An unsolicited connect-time event first, then the reply.
                let _ = write
                    .write_all(b"{\"type\":\"workspace\",\"path\":\"/w\"}\n")
                    .await;
                let reply = serde_json::json!({
                    "type": "response",
                    "id": request["id"],
                    "command": request["type"],
                    "success": ok,
                    "error": if ok { serde_json::Value::Null } else { "harness already terminated".into() },
                });
                let _ = write.write_all(format!("{reply}\n").as_bytes()).await;
                // Keep the connection until the client is done.
                let _ = quecto_line_io::read_frame_or_legacy_line(&mut reader, 1024).await;
            });
        }
    });
    (path, requests)
}

fn registry_with(
    uuid: &str,
    socket: &std::path::Path,
    generation: Option<u64>,
) -> SubagentRegistry {
    let registry: SubagentRegistry = Arc::new(Mutex::new(Default::default()));
    let mut entry = SubagentEntry::with_identity(
        AgentUuid::new(uuid),
        uuid.to_owned(),
        socket.to_path_buf(),
        0,
    );
    entry.launch_generation = generation.map(LaunchGeneration::new);
    registry.lock().unwrap().insert(uuid.to_owned(), entry);
    registry
}

fn identity(uuid: &str, generation: u64) -> DelegatedAgentIdentity {
    DelegatedAgentIdentity::new(uuid, LaunchGeneration::new(generation))
}

#[tokio::test]
async fn shutdown_child_sends_one_correlated_command_and_returns_on_ack() {
    let (socket, requests) = fake_child(true);
    let routing = UdsDirectChildRouting::new(registry_with("A", &socket, Some(3)));
    routing
        .shutdown_child(&identity("A", 3), ShutdownReason::ParentShutdown)
        .await
        .unwrap();
    let seen = requests.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0]["type"], "shutdown");
    assert_eq!(seen[0]["reason"], "parent_shutdown");
    assert!(seen[0]["id"].is_string());
}

#[tokio::test]
async fn forward_termination_carries_target_generation_and_depth() {
    let (socket, requests) = fake_child(true);
    let routing = UdsDirectChildRouting::new(registry_with("A", &socket, Some(1)));
    routing
        .forward_termination(
            &identity("A", 1),
            &identity("B", 4),
            RoutingDepth::new(2).unwrap(),
        )
        .await
        .unwrap();
    let seen = requests.lock().unwrap().clone();
    assert_eq!(seen[0]["type"], "terminate_delegated_agent");
    assert_eq!(seen[0]["target_uuid"], "B");
    assert_eq!(seen[0]["target_generation"], 4);
    assert_eq!(seen[0]["remaining_depth"], 2);
}

#[tokio::test]
async fn only_a_launched_current_generation_child_is_direct() {
    let (socket, requests) = fake_child(true);
    let routing = UdsDirectChildRouting::new(registry_with("A", &socket, Some(2)));
    for (uuid, generation) in [("A", 1), ("A", 3), ("ghost", 2)] {
        assert_eq!(
            routing
                .shutdown_child(&identity(uuid, generation), ShutdownReason::ParentShutdown)
                .await,
            Err(ChildRoutingError::NotADirectChild)
        );
    }
    // A merged/restored row (no generation) is never direct either.
    let merged = UdsDirectChildRouting::new(registry_with("M", &socket, None));
    assert_eq!(
        merged
            .shutdown_child(&identity("M", 1), ShutdownReason::ParentShutdown)
            .await,
        Err(ChildRoutingError::NotADirectChild)
    );
    assert!(requests.lock().unwrap().is_empty(), "no edge was touched");
}

#[tokio::test]
async fn refusals_and_unreachable_endpoints_are_negative_outcomes() {
    let (refusing, _) = fake_child(false);
    let routing = UdsDirectChildRouting::new(registry_with("A", &refusing, Some(1)))
        .with_timeout(Duration::from_secs(2));
    assert_eq!(
        routing
            .shutdown_child(&identity("A", 1), ShutdownReason::OperatorRequest)
            .await,
        Err(ChildRoutingError::Unreachable(
            "harness already terminated".into()
        ))
    );
    let missing = std::env::temp_dir().join("q-dcr-missing.sock");
    let outcome = shutdown_protocol_attempt(
        missing,
        ShutdownReason::ParentShutdown,
        Duration::from_secs(1),
    )
    .await;
    assert!(matches!(outcome, ProtocolOutcome::Negative(detail) if detail.contains("connect")));
    let empty = shutdown_protocol_attempt(
        std::path::PathBuf::new(),
        ShutdownReason::ParentShutdown,
        Duration::from_secs(1),
    )
    .await;
    assert_eq!(
        empty,
        ProtocolOutcome::Negative("unreachable: no endpoint".into())
    );
    let (ok, _) = fake_child(true);
    assert_eq!(
        shutdown_protocol_attempt(ok, ShutdownReason::ParentShutdown, Duration::from_secs(2)).await,
        ProtocolOutcome::Acknowledged
    );
}
