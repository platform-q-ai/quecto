//! #2681 round 1: the kill's kept-environments reader is composed over the
//! very registry the teardown retains through. A direct container child is
//! killed through the composed `agent_cmd kill` tool; its row's exit is
//! observed (as the monitor's EOF is), its environment is kept by the
//! composed final-member teardown, and the tool's answer names it.
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::build_termination_owners;
use crate::application::subagents::ports::delegated_agent_registry::{
    DelegatedAgentRegistry, ExitObservation, TerminalClaim, TerminationCause,
};
use crate::application::subagents::ports::teardown_compensation::TeardownCompensation;
use crate::domain::agents::services::subagent_teardown::{
    DelegatedAgentIdentity, LaunchGeneration,
};
use crate::domain::environments::entities::environment_registry::{
    EnvironmentOrigin, EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus,
};
use crate::domain::identity::ids::AgentUuid;
use crate::infrastructure::tools::agents::environment_member_shutdown::TerminationSlots;
use crate::infrastructure::tools::agents::subagent_registry::{SubagentEntry, SubagentRegistry};
use crate::infrastructure::tools::agents::subagent_teardown_registry::RegistryDelegatedAgents;
use crate::interface::cli::KillToolWiring;

/// A direct child on a socket that acknowledges every command, counting
/// what it was sent.
fn acknowledging_child() -> (std::path::PathBuf, Arc<Mutex<usize>>) {
    let path = std::env::temp_dir().join(format!("q-kept-{}.sock", uuid::Uuid::new_v4().simple()));
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let asked = Arc::new(Mutex::new(0));
    let counted = asked.clone();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let counted = counted.clone();
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
                *counted.lock().unwrap() += 1;
                let reply = serde_json::json!({
                    "type": "response", "id": request["id"], "command": request["type"],
                    "success": true,
                });
                let _ = write.write_all(format!("{reply}\n").as_bytes()).await;
                let _ = quecto_line_io::read_frame_or_legacy_line(&mut reader, 1024).await;
            });
        }
    });
    (path, asked)
}

#[tokio::test]
async fn a_kill_whose_teardown_keeps_its_environment_names_it() {
    let base = tempfile::tempdir().unwrap();
    let (socket, asked) = acknowledging_child();
    let registry: SubagentRegistry = Arc::new(Mutex::new(Default::default()));
    let environments = EnvironmentRegistry::new();
    let env_ref = environments.mint_ref().unwrap();
    environments.commit(EnvironmentRecord {
        environment_ref: env_ref.clone(),
        environment_id: "env-kept".into(),
        environment_uuid: "uuid-kept".into(),
        name: None,
        // Not on this host and no advertised checkout: the owner's end of
        // its only member keeps it (#2206 round 3).
        workspace_path: base.path().join("in-container-only"),
        repository: String::new(),
        script_name: "default".into(),
        retained_exec_argv: vec![],
        retained_kill_argv: vec!["true".into()],
        retained_cleanup_argv: vec![],
        retained_inspect_argv: vec![],
        members: vec![],
        status: EnvironmentStatus::Running,
        metadata: serde_json::json!({}),
        last_error: None,
        origin: EnvironmentOrigin::Created,
        created_by: String::new(),
        created_at: None,
    });
    {
        let mut entry =
            SubagentEntry::with_identity(AgentUuid::new("A"), "A".into(), socket.clone(), 0);
        entry.launch_generation = Some(LaunchGeneration::new(1));
        entry.environment_registry = Some(environments.clone());
        entry.environment_ref = Some(env_ref.clone());
        environments.add_member(&env_ref, "A").unwrap();
        registry.lock().unwrap().insert("A".into(), entry);
    }
    let owners = build_termination_owners(KillToolWiring {
        owner: AgentUuid::new("root"),
        registry: registry.clone(),
        broadcast_tx: None,
        notify_tx: None,
        harness_lifecycle:
            crate::infrastructure::tools::agents::harness_lifecycle::new_shared_harness_lifecycle(),
        environment_registry: environments.clone(),
        slots: TerminationSlots::default(),
        base_dir: base.path().to_path_buf(),
    });
    // The row's exit, observed as the monitor observes its EOF once the
    // child acknowledged: claimed and compensated over the same registry.
    let observer = {
        let agents = RegistryDelegatedAgents::new(
            registry.clone(),
            None,
            None,
            super::super::environments::build_member_finalizer,
        );
        let asked = asked.clone();
        tokio::spawn(async move {
            while *asked.lock().unwrap() == 0 {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let child = DelegatedAgentIdentity::new("A", LaunchGeneration::new(1));
            assert_eq!(agents.claim_terminal(&child), TerminalClaim::Claimed);
            agents
                .compensate(
                    &child,
                    TerminationCause::Exit(ExitObservation::ConnectionClosed),
                )
                .await;
        })
    };
    let answer = tokio::time::timeout(
        Duration::from_secs(20),
        owners.kill_tool.execute(r#"{"agent_id":"A"}"#),
    )
    .await
    .expect("bounded")
    .unwrap();
    observer.await.unwrap();
    let _ = std::fs::remove_file(&socket);
    assert!(!answer.is_error, "{}", answer.content);
    let body: serde_json::Value = serde_json::from_str(&answer.content).unwrap();
    let record = environments.get(&env_ref).unwrap();
    assert_eq!(record.status, EnvironmentStatus::Retained);
    let reason = record.retained_reason().expect("a kept box says why");
    assert_eq!(
        body["kept"],
        serde_json::json!([{"environment": env_ref, "reason": reason}]),
        "{body}"
    );
}
