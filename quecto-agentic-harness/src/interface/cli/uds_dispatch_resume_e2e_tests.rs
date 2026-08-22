use crate::domain::message::Message;
use crate::domain::session::{
    PersistedSubagentRosterEntry, Session, SessionStore, SubagentLiveness,
};
use crate::infrastructure::tools::subagent_registry::new_registry;
use crate::interface::cli::protocol::AgentCommand;

use super::cov_tests::Fixture;

#[tokio::test]
async fn e2e_resume_picker_lists_persisted_default_tui_chat_session() {
    let mut fx = Fixture::new();
    let persisted_key = crate::domain::session::Session::build_key("cli", "default");
    fx.store
        .save(&Session {
            key: persisted_key.clone(),
            messages: vec![Message::user("persisted message that /resume must offer")],
            workflow_run: None,
            subagent_roster: Vec::new(),
        })
        .await
        .unwrap();

    let listed = fx.store.list(None).await.unwrap();
    assert!(
        listed.iter().any(|summary| summary.key == persisted_key),
        "a TUI-owned persisted default session must be offered by bare /resume; listed={listed:?}"
    );

    let mut ctx = fx.ctx();
    assert!(
        !super::dispatch_command(
            AgentCommand::ListSessions {
                id: Some("resume-list".into()),
            },
            &mut ctx,
        )
        .await
    );
    assert!(
        !super::handle_resume_session(
            &mut ctx,
            Some("resume-select"),
            "resume_session",
            persisted_key.clone(),
        )
        .await
    );
    assert_eq!(*ctx.session_key, persisted_key);
    assert_eq!(ctx.messages.len(), 1);
}

fn persisted_roster_entry(
    id: &str,
    socket_path: std::path::PathBuf,
    liveness: SubagentLiveness,
) -> PersistedSubagentRosterEntry {
    PersistedSubagentRosterEntry {
        agent_uuid: id.to_string(),
        display_name: format!("worker-{id}"),
        session_key: id.to_string(),
        socket_path,
        pid: 1,
        liveness,
        parent_id: Some("parent".to_string()),
        read_only: true,
        delivered_message_ordinal: None,
        pending_message_reports: std::collections::VecDeque::new(),
        status: Some("idle".to_string()),
    }
}

fn serve_matching_session_stats(
    listener: std::os::unix::net::UnixListener,
    session_key: String,
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

#[tokio::test]
async fn e2e_resume_restores_transcript_and_prunes_subagent_roster() {
    let mut fx = Fixture::new();
    let dir = tempfile::tempdir().unwrap();
    let live_socket = dir.path().join("live.sock");
    let detached_socket = dir.path().join("detached.sock");
    let live_listener = std::os::unix::net::UnixListener::bind(&live_socket).unwrap();
    let detached_listener = std::os::unix::net::UnixListener::bind(&detached_socket).unwrap();
    let live_server = serve_matching_session_stats(live_listener, "live".to_string());
    let detached_server = serve_matching_session_stats(detached_listener, "detached".to_string());

    fx.store
        .save(&Session {
            key: "cli:roster-resume".to_string(),
            messages: vec![
                Message::user("persisted transcript survives roster restore"),
                Message::assistant("persisted answer", vec![]),
            ],
            workflow_run: None,
            subagent_roster: vec![
                persisted_roster_entry("live", live_socket, SubagentLiveness::Live),
                persisted_roster_entry(
                    "unreachable",
                    dir.path().join("gone.sock"),
                    SubagentLiveness::Live,
                ),
                persisted_roster_entry(
                    "dead",
                    dir.path().join("dead.sock"),
                    SubagentLiveness::Dead,
                ),
                persisted_roster_entry("detached", detached_socket, SubagentLiveness::Detached),
                persisted_roster_entry(
                    "detached-gone",
                    dir.path().join("old.sock"),
                    SubagentLiveness::Detached,
                ),
            ],
        })
        .await
        .unwrap();

    let registry = new_registry();
    {
        let mut ctx = fx.ctx();
        ctx.subagent_registry = Some(registry.clone());
        assert!(
            !super::handle_resume_session(
                &mut ctx,
                Some("resume-roster"),
                "resume_session",
                "roster-resume".into(),
            )
            .await
        );
    }

    assert_eq!(fx.session_key, "cli:roster-resume");
    assert_eq!(fx.messages.len(), 2);
    assert_eq!(
        fx.messages[0].content,
        "persisted transcript survives roster restore"
    );
    assert_eq!(fx.messages[1].content, "persisted answer");

    let entries = registry.lock().unwrap();
    let mut restored = entries.keys().cloned().collect::<Vec<_>>();
    restored.sort();
    assert_eq!(
        restored,
        vec!["detached".to_string(), "live".to_string()],
        "resume should restore only currently verifiable live/detached roster entries"
    );
    assert_eq!(
        entries.get("detached").unwrap().persisted_liveness,
        SubagentLiveness::Live,
        "reachable detached entries are restored as live; dead and unreachable rows are pruned"
    );

    drop(entries);
    live_server.join().unwrap();
    detached_server.join().unwrap();
}
