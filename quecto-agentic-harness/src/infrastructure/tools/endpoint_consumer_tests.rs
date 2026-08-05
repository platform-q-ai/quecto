use super::subagent_registry::{SubagentEntry, SubagentStatus, new_registry};
use crate::domain::agent_launch_backend::ParentEndpoint;
use crate::domain::ids::AgentUuid;
use crate::domain::tool::Tool;
use crate::infrastructure::tools::agent_cmd::AgentCmdTool;
use std::path::PathBuf;

fn proxy_entry(id: &str, requested: PathBuf, proxy: PathBuf) -> SubagentEntry {
    let mut entry = SubagentEntry::with_identity(AgentUuid::new(id), id.to_string(), requested, 0);
    entry.status = SubagentStatus::Idle;
    entry.socket_mode = Some("proxy".to_string());
    entry.parent_endpoint = Some(ParentEndpoint::Proxy(format!("unix:{}", proxy.display())));
    entry
}

async fn one_shot_server(path: PathBuf, tag: &'static str) -> tokio::task::JoinHandle<String> {
    let _ = std::fs::remove_file(&path);
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = tokio::io::BufReader::new(stream);
        let payload = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            quecto_line_io::read_frame(&mut reader, quecto_line_io::PROTOCOL_FRAME_CAP_BYTES),
        )
        .await;
        let Ok(Ok(Some(payload))) = payload else {
            return String::new();
        };
        let line = String::from_utf8(payload).unwrap();
        let sent: serde_json::Value = serde_json::from_str(&line).unwrap();
        let ack = serde_json::json!({"type":"response","id":sent["id"],"success":true,"tag":tag})
            .to_string();
        quecto_line_io::write_frame(
            reader.get_mut(),
            ack.as_bytes(),
            quecto_line_io::PROTOCOL_FRAME_CAP_BYTES,
        )
        .await
        .unwrap();
        line
    })
}

#[tokio::test]
async fn proxy_without_validated_endpoint_fails_command_and_await_without_direct_contact() {
    let registry = new_registry();
    let dir = tempfile::tempdir().unwrap();
    let requested = dir.path().join("requested.sock");
    let direct = one_shot_server(requested.clone(), "direct").await;
    let mut entry =
        SubagentEntry::with_identity(AgentUuid::new("agent-a"), "agent-a".into(), requested, 0);
    entry.status = SubagentStatus::Idle;
    entry.socket_mode = Some("proxy".into());
    registry.lock().unwrap().insert("agent-a".into(), entry);
    let tool = AgentCmdTool::new(registry);

    let command = tool
        .execute(r#"{"agent_id":"agent-a","command":"get_state"}"#)
        .await
        .unwrap();
    assert!(command.is_error);
    assert!(command.content.contains("no validated proxy endpoint"));

    let awaited = tool
        .execute(r#"{"agent_id":"agent-a","command":"await","idle_timeout":0}"#)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&awaited.content).unwrap()["reason"],
        "endpoint_unavailable"
    );
    assert!(
        !direct.is_finished(),
        "direct requested socket must not be contacted"
    );
    direct.abort();
}

#[tokio::test]
async fn shared_requested_socket_uses_each_agents_validated_proxy_endpoint() {
    let registry = new_registry();
    let dir = tempfile::tempdir().unwrap();
    let requested = dir.path().join("same.sock");
    let proxy_a = dir.path().join("a.sock");
    let proxy_b = dir.path().join("b.sock");
    let server_a = one_shot_server(proxy_a.clone(), "a").await;
    let server_b = one_shot_server(proxy_b.clone(), "b").await;
    registry.lock().unwrap().insert(
        "agent-a".into(),
        proxy_entry("agent-a", requested.clone(), proxy_a),
    );
    registry
        .lock()
        .unwrap()
        .insert("agent-b".into(), proxy_entry("agent-b", requested, proxy_b));
    let tool = AgentCmdTool::new(registry);

    let a = tool
        .execute(r#"{"agent_id":"agent-a","command":"get_state"}"#)
        .await
        .unwrap();
    assert!(a.content.contains(r#""tag":"a""#), "{}", a.content);
    assert!(
        !server_b.is_finished(),
        "agent-a must not contact agent-b endpoint"
    );
    let b = tool
        .execute(r#"{"agent_id":"agent-b","command":"get_state"}"#)
        .await
        .unwrap();
    assert!(b.content.contains(r#""tag":"b""#), "{}", b.content);
    let _ = server_a.await;
    let _ = server_b.await;
}

#[tokio::test]
async fn valid_proxy_and_direct_await_behavior_remain_supported() {
    let registry = new_registry();
    let dir = tempfile::tempdir().unwrap();
    let proxy = dir.path().join("proxy.sock");
    let direct_path = dir.path().join("direct.sock");
    let _proxy_server = one_shot_server(proxy.clone(), "proxy").await;
    let _direct_server = one_shot_server(direct_path.clone(), "direct").await;
    registry.lock().unwrap().insert(
        "proxy".into(),
        proxy_entry("proxy", dir.path().join("requested.sock"), proxy),
    );
    let mut direct =
        SubagentEntry::with_identity(AgentUuid::new("direct"), "direct".into(), direct_path, 0);
    direct.status = SubagentStatus::Idle;
    registry.lock().unwrap().insert("direct".into(), direct);
    let tool = AgentCmdTool::new(registry);

    for id in ["proxy", "direct"] {
        let result = tool
            .execute(&format!(
                r#"{{"agent_id":"{id}","command":"await","idle_timeout":0}}"#
            ))
            .await
            .unwrap();
        assert!(!result.is_error, "{id}: {}", result.content);
    }
}
