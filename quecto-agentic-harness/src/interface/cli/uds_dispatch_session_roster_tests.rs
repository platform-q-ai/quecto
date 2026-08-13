use crate::domain::ids::AgentUuid;
use crate::domain::session::{PersistedSubagentRosterEntry, SubagentLiveness};
use crate::infrastructure::tools::subagent_registry::{SubagentEntry, new_registry};
use crate::interface::cli::uds::uds_dispatch_session::{
    restore_persisted_subagent_roster, snapshot_subagent_roster, verify_persisted_live_subagent,
};

fn roster_entry(id: &str, socket_path: std::path::PathBuf) -> PersistedSubagentRosterEntry {
    PersistedSubagentRosterEntry {
        agent_uuid: id.to_string(),
        display_name: format!("worker-{id}"),
        session_key: id.to_string(),
        socket_path,
        pid: 1,
        liveness: SubagentLiveness::Live,
        parent_id: Some("parent".to_string()),
        read_only: true,
        status: Some("idle".to_string()),
    }
}

#[test]
fn snapshot_subagent_roster_serializes_sorted_liveness_metadata() {
    let registry = new_registry();
    {
        let mut entries = registry.lock().unwrap();
        let mut b = SubagentEntry::with_identity(
            AgentUuid::from("b".to_string()),
            "beta".to_string(),
            "/tmp/b.sock".into(),
            20,
        );
        b.persisted_liveness = SubagentLiveness::Dead;
        b.read_only = true;
        b.parent_id = Some("root".to_string());
        entries.insert("b".to_string(), b);
        let mut a = SubagentEntry::with_identity(
            AgentUuid::from("a".to_string()),
            "alpha".to_string(),
            "/tmp/a.sock".into(),
            10,
        );
        a.persisted_liveness = SubagentLiveness::Detached;
        entries.insert("a".to_string(), a);
    }

    let roster = snapshot_subagent_roster(&Some(registry));
    assert_eq!(roster.len(), 2);
    assert_eq!(roster[0].agent_uuid, "a");
    assert_eq!(roster[0].display_name, "alpha");
    assert_eq!(roster[0].liveness, SubagentLiveness::Detached);
    assert_eq!(roster[1].agent_uuid, "b");
    assert_eq!(roster[1].display_name, "beta");
    assert_eq!(roster[1].liveness, SubagentLiveness::Dead);
    assert_eq!(roster[1].parent_id.as_deref(), Some("root"));
    assert!(roster[1].read_only);
}

#[test]
fn verify_persisted_live_subagent_rejects_empty_uuid_or_socket() {
    assert!(!verify_persisted_live_subagent(&roster_entry(
        "",
        "/tmp/live.sock".into()
    )));
    assert!(!verify_persisted_live_subagent(&roster_entry(
        "child",
        "".into()
    )));
}

#[tokio::test]
async fn verify_persisted_live_subagent_requires_matching_session_stats_identity() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("child.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (reader, mut writer) = tokio::io::split(stream);
        let mut reader = tokio::io::BufReader::new(reader);
        let request =
            quecto_line_io::read_frame(&mut reader, quecto_line_io::PROTOCOL_FRAME_CAP_BYTES)
                .await
                .unwrap()
                .unwrap();
        let request_id = serde_json::from_slice::<serde_json::Value>(&request).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        let response = serde_json::json!({
            "type": "response",
            "id": request_id,
            "command": "get_session_stats",
            "success": true,
            "data": { "sessionKey": "cli:other", "userMessages": 0, "assistantMessages": 0, "toolCalls": 0, "toolResults": 0, "totalMessages": 0, "tokens": {}, "contextTokens": 0, "maxContextTokens": 0 }
        });
        quecto_line_io::write_frame(
            &mut writer,
            response.to_string().as_bytes(),
            quecto_line_io::PROTOCOL_FRAME_CAP_BYTES,
        )
        .await
        .unwrap();
    });

    let mut entry = roster_entry("child", socket);
    entry.session_key = "cli:child".into();
    assert!(
        !tokio::task::spawn_blocking(move || verify_persisted_live_subagent(&entry))
            .await
            .unwrap()
    );
    server.await.unwrap();
}

/// #1474: resume must not rehydrate dead / unverifiable roster rows as grey
/// "ghost" panel agents. Only currently verifiable live agents re-enter the
/// registry; dead and failed-verify live/detached entries are pruned.
fn serve_matching_session_stats(
    listener: std::os::unix::net::UnixListener,
    session_key: &'static str,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        let (mut stream, _) = listener.accept().unwrap();
        let mut prefix = [0u8; 4];
        stream.read_exact(&mut prefix).unwrap();
        let len = u32::from_be_bytes(prefix) as usize;
        let mut request = vec![0u8; len];
        stream.read_exact(&mut request).unwrap();
        let request_id = serde_json::from_slice::<serde_json::Value>(&request).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        let response = serde_json::json!({
            "type": "response",
            "id": request_id,
            "command": "get_session_stats",
            "success": true,
            "data": {
                "sessionKey": session_key,
                "userMessages": 0,
                "assistantMessages": 0,
                "toolCalls": 0,
                "toolResults": 0,
                "totalMessages": 0,
                "tokens": {},
                "contextTokens": 0,
                "maxContextTokens": 0
            }
        })
        .to_string();
        stream
            .write_all(&(response.len() as u32).to_be_bytes())
            .unwrap();
        stream.write_all(response.as_bytes()).unwrap();
        stream.flush().unwrap();
    })
}

#[test]
fn restore_persisted_roster_keeps_only_verifiably_live_agents() {
    let registry = new_registry();
    let dir = tempfile::tempdir().unwrap();
    let live_socket = dir.path().join("live.sock");
    let detached_live_socket = dir.path().join("detached-live.sock");
    let live_listener = std::os::unix::net::UnixListener::bind(&live_socket).unwrap();
    let detached_listener = std::os::unix::net::UnixListener::bind(&detached_live_socket).unwrap();
    let live_server = serve_matching_session_stats(live_listener, "live");
    let detached_server = serve_matching_session_stats(detached_listener, "still-up");

    let mut live = roster_entry("live", live_socket);
    live.display_name = "Live worker".into();
    live.read_only = false;
    let mut unreachable = roster_entry("gone", dir.path().join("gone.sock"));
    unreachable.display_name = "Gone worker".into();
    let mut dead = roster_entry("dead", dir.path().join("dead.sock"));
    dead.liveness = SubagentLiveness::Dead;
    let mut detached_unreachable = roster_entry("old-container", dir.path().join("old.sock"));
    detached_unreachable.liveness = SubagentLiveness::Detached;
    detached_unreachable.display_name = "Old container ghost".into();
    let mut detached_live = roster_entry("still-up", detached_live_socket);
    detached_live.liveness = SubagentLiveness::Detached;
    detached_live.display_name = "Detached but reachable".into();

    restore_persisted_subagent_roster(
        &Some(registry.clone()),
        vec![live, unreachable, dead, detached_unreachable, detached_live],
    );

    let entries = registry.lock().unwrap();
    assert_eq!(
        entries.len(),
        2,
        "dead and unverifiable live/detached entries must not reappear as ghosts: {:?}",
        entries.keys().collect::<Vec<_>>()
    );
    assert!(
        !entries.contains_key("gone"),
        "unreachable live entry must be pruned on restore"
    );
    assert!(
        !entries.contains_key("dead"),
        "dead entry must be pruned on restore"
    );
    assert!(
        !entries.contains_key("old-container"),
        "detached unreachable entry must be pruned on restore"
    );

    let live_entry = entries.get("live").expect("verified live agent restored");
    assert_eq!(live_entry.persisted_liveness, SubagentLiveness::Live);
    assert_eq!(live_entry.status.to_wire_str(), "idle");
    assert_eq!(live_entry.display_name, "Live worker");

    let detached_entry = entries
        .get("still-up")
        .expect("verified detached-but-reachable agent restored as live");
    assert_eq!(detached_entry.persisted_liveness, SubagentLiveness::Live);
    assert_eq!(detached_entry.status.to_wire_str(), "idle");
    assert_eq!(detached_entry.display_name, "Detached but reachable");

    live_server.join().unwrap();
    detached_server.join().unwrap();
}

#[test]
fn restore_persisted_roster_no_registry_is_noop() {
    restore_persisted_subagent_roster(&None, vec![roster_entry("ignored", "".into())]);
}
