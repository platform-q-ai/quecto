//! #843 child-targeted `agent_cmd` dispatch/forwarding characterization.
use super::super::uds_dispatch_sync_forward::forward_subagent_sync;
use super::{
    AgentCommand, ForwardGetMessage, dispatch_command, forward_subagent_get_message,
    forward_subagent_get_messages,
};
use crate::domain::message::Message;
use crate::infrastructure::tools::subagent_registry::{
    SubagentEntry, SubagentStatus, new_registry,
};

use super::tests_843::{
    Fx, register_child, spawn_recording_child, spawn_recording_get_message_child,
    spawn_recording_replying_child, spawn_recording_sync_child, spawn_replying_child,
};

#[tokio::test]
async fn dispatch_tail_agent_targeted_get_messages_forwards_count_to_child() {
    let (sock, received, _dir, handle) = spawn_recording_child("TAIL_CHILD_HISTORY").await;
    let registry = new_registry();
    register_child(&registry, "worker", sock);

    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(8);
    let mut fx = Fx::new();
    fx.messages.push(Message::user("PARENT_ONLY"));
    {
        let mut ctx = fx.ctx();
        ctx.subagent_registry = Some(registry);
        ctx.broadcast_tx = Some(tx);
        let cmd = AgentCommand::GetMessagesTail {
            id: Some("tail-q".into()),
            count: 3,
            agent_id: Some("worker".into()),
        };
        assert!(!dispatch_command(cmd, &mut ctx).await);
    }
    handle.await.unwrap();

    let emitted = rx.try_recv().expect("a response event should be emitted");
    assert!(emitted.contains("TAIL_CHILD_HISTORY"), "got: {emitted}");
    assert!(!emitted.contains("PARENT_ONLY"), "got: {emitted}");
    let fwd = received.lock().await.clone();
    let fwd_json: serde_json::Value = serde_json::from_str(&fwd).unwrap();
    assert_eq!(fwd_json["type"], "get_messages");
    assert_eq!(fwd_json["count"], 3);
}

#[tokio::test]
async fn dispatch_agent_targeted_get_message_forwards_to_child_before_parent_history() {
    let (sock, received, _dir, handle) = spawn_recording_get_message_child("CHILD_MESSAGE").await;
    let registry = new_registry();
    register_child(&registry, "worker", sock);

    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(8);
    let mut fx = Fx::new();
    fx.messages.push(Message::user("PARENT_ONLY"));
    {
        let mut ctx = fx.ctx();
        ctx.subagent_registry = Some(registry);
        ctx.broadcast_tx = Some(tx);
        let cmd = AgentCommand::GetMessage {
            id: Some("gm-q".into()),
            message_id: "child-message".into(),
            agent_id: Some("worker".into()),
            tool_call_id: None,
            offset: Some(1),
            thinking_offset: None,
            limit: Some(4),
        };
        assert!(!dispatch_command(cmd, &mut ctx).await);
    }
    handle.await.unwrap();

    let emitted = rx.try_recv().expect("a response event should be emitted");
    let json: serde_json::Value = serde_json::from_str(&emitted).unwrap();
    assert_eq!(json["success"], true);
    assert_eq!(json["data"]["content"], "CHILD_MESSAGE");
    let fwd = received.lock().await.clone();
    let fwd_json: serde_json::Value = serde_json::from_str(&fwd).unwrap();
    assert_eq!(fwd_json["type"], "get_message");
    assert_eq!(fwd_json["messageId"], "child-message");
    assert_eq!(fwd_json["offset"], 1);
    assert_eq!(fwd_json["limit"], 4);
}

#[tokio::test]
async fn dispatch_agent_targeted_sync_forwards_to_child_before_parent_snapshot() {
    let (sock, received, _dir, handle) = spawn_recording_sync_child("child-sync-id").await;
    let registry = new_registry();
    register_child(&registry, "worker", sock);

    let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(8);
    let mut fx = Fx::new();
    {
        let mut ctx = fx.ctx();
        ctx.subagent_registry = Some(registry);
        ctx.broadcast_tx = Some(tx);
        let cmd = AgentCommand::Sync {
            id: Some("sync-q".into()),
            epoch: 4,
            since_rev: 2,
            agent_id: Some("worker".into()),
        };
        assert!(!dispatch_command(cmd, &mut ctx).await);
    }
    handle.await.unwrap();

    let emitted = rx.try_recv().expect("a response event should be emitted");
    let json: serde_json::Value = serde_json::from_str(&emitted).unwrap();
    assert_eq!(json["success"], true);
    assert_eq!(json["data"]["changes"][0]["id"], "child-sync-id");
    let fwd = received.lock().await.clone();
    let fwd_json: serde_json::Value = serde_json::from_str(&fwd).unwrap();
    assert_eq!(fwd_json["type"], "sync");
    assert_eq!(fwd_json["epoch"], 4);
    assert_eq!(fwd_json["sinceRev"], 2);
}

#[tokio::test]
async fn forward_sync_propagates_child_failure() {
    let (sock, received, _dir, handle) = spawn_recording_replying_child(
        "{\"type\":\"response\",\"id\":\"__ID__\",\"command\":\"sync\",\"success\":false,\"error\":\"sync cursor expired\"}\n",
    )
    .await;
    let registry = new_registry();
    register_child(&registry, "worker", sock);
    let mut fx = Fx::new();
    let mut ctx = fx.ctx();
    ctx.subagent_registry = Some(registry);

    let event = forward_subagent_sync(
        &ctx,
        Some(crate::domain::ids::CommandId::from("sync-2")),
        "sync",
        crate::domain::ids::AgentId::from("worker"),
        7,
        3,
    )
    .await;
    handle.await.unwrap();
    let json = serde_json::to_value(event).unwrap();

    assert_eq!(json["success"], false);
    assert_eq!(json["error"], "sync cursor expired");
    let fwd = received.lock().await.clone();
    let fwd_json: serde_json::Value = serde_json::from_str(&fwd).unwrap();
    assert_eq!(fwd_json["type"], "sync");
    assert_eq!(fwd_json["epoch"], 7);
    assert_eq!(fwd_json["sinceRev"], 3);
}

#[tokio::test]
async fn forward_get_message_propagates_child_failure() {
    let (sock, _dir, handle) = spawn_replying_child(
        "{\"type\":\"response\",\"id\":\"__ID__\",\"command\":\"get_message\",\"success\":false,\"error\":\"message not found: child-missing\"}\n",
    )
    .await;
    let registry = new_registry();
    register_child(&registry, "worker", sock);
    let mut fx = Fx::new();
    let mut ctx = fx.ctx();
    ctx.subagent_registry = Some(registry);

    let ev = forward_subagent_get_message(
        &ctx,
        Some("parent-page"),
        "get_message",
        ForwardGetMessage {
            agent_id: crate::domain::ids::AgentId::from("worker"),
            message_id: crate::domain::ids::MessageId::from("child-missing"),
            tool_call_id: None,
            offset: None,
            thinking_offset: None,
            limit: None,
        },
    )
    .await;
    handle.await.unwrap();
    let json = serde_json::to_value(&ev).unwrap();
    assert_eq!(json["success"], false);
    assert_eq!(json["error"], "message not found: child-missing");
}

#[tokio::test]
async fn forward_get_message_rejects_wrong_command_child_response() {
    let (sock, _dir, handle) = spawn_replying_child(
        "{\"type\":\"response\",\"id\":\"__ID__\",\"command\":\"get_messages\",\"success\":true,\"data\":{\"messages\":[]}}\n",
    )
    .await;
    let registry = new_registry();
    register_child(&registry, "worker", sock);
    let mut fx = Fx::new();
    let mut ctx = fx.ctx();
    ctx.subagent_registry = Some(registry);

    let ev = forward_subagent_get_message(
        &ctx,
        Some("parent-page"),
        "get_message",
        ForwardGetMessage {
            agent_id: crate::domain::ids::AgentId::from("worker"),
            message_id: crate::domain::ids::MessageId::from("m1"),
            tool_call_id: None,
            offset: None,
            thinking_offset: None,
            limit: None,
        },
    )
    .await;
    handle.await.unwrap();
    let json = serde_json::to_value(&ev).unwrap();
    assert_eq!(json["success"], false);
    assert_eq!(json["error"], "unexpected child response command");
}

#[tokio::test]
async fn forward_get_message_rejects_missing_data_child_response() {
    let (sock, _dir, handle) = spawn_replying_child(
        "{\"type\":\"response\",\"id\":\"__ID__\",\"command\":\"get_message\",\"success\":true}\n",
    )
    .await;
    let registry = new_registry();
    register_child(&registry, "worker", sock);
    let mut fx = Fx::new();
    let mut ctx = fx.ctx();
    ctx.subagent_registry = Some(registry);

    let ev = forward_subagent_get_message(
        &ctx,
        Some("parent-page"),
        "get_message",
        ForwardGetMessage {
            agent_id: crate::domain::ids::AgentId::from("worker"),
            message_id: crate::domain::ids::MessageId::from("m1"),
            tool_call_id: None,
            offset: None,
            thinking_offset: None,
            limit: None,
        },
    )
    .await;
    handle.await.unwrap();
    let json = serde_json::to_value(&ev).unwrap();
    assert_eq!(json["success"], false);
    assert_eq!(json["error"], "get_message response missing data");
}

#[tokio::test]
async fn forward_get_message_rejects_malformed_child_response() {
    let (sock, _dir, handle) = spawn_replying_child("not-json\n").await;
    let registry = new_registry();
    register_child(&registry, "worker", sock);
    let mut fx = Fx::new();
    let mut ctx = fx.ctx();
    ctx.subagent_registry = Some(registry);

    let ev = forward_subagent_get_message(
        &ctx,
        Some("parent-page"),
        "get_message",
        ForwardGetMessage {
            agent_id: crate::domain::ids::AgentId::from("worker"),
            message_id: crate::domain::ids::MessageId::from("m1"),
            tool_call_id: None,
            offset: None,
            thinking_offset: None,
            limit: None,
        },
    )
    .await;
    handle.await.unwrap();
    let json = serde_json::to_value(&ev).unwrap();
    assert_eq!(json["success"], false);
    assert!(
        json["error"]
            .as_str()
            .is_some_and(|error| !error.is_empty()),
        "malformed child JSON must be surfaced as an error: {json}"
    );
}

#[tokio::test]
async fn forward_get_messages_rejects_stale_historical_before_cursor() {
    use crate::domain::session::{Session, SessionStore, SubagentLiveness};

    let registry = new_registry();
    {
        let mut entry = SubagentEntry::with_identity(
            crate::domain::ids::AgentUuid::from("dead-child"),
            "dead-label".into(),
            "/tmp/dead.sock".into(),
            9,
        );
        entry.status = SubagentStatus::Exited;
        entry.persisted_liveness = SubagentLiveness::Dead;
        registry.lock().unwrap().insert("dead-child".into(), entry);
    }
    let mut fx = Fx::new();
    fx.store
        .save(&Session {
            key: "dead-child".into(),
            messages: vec![Message::user("historical transcript")],
            workflow_run: None,
            subagent_roster: Vec::new(),
        })
        .await
        .unwrap();
    let mut ctx = fx.ctx();
    ctx.subagent_registry = Some(registry);

    let ev = forward_subagent_get_messages(
        &ctx,
        Some(crate::domain::ids::CommandId::from("stale-before")),
        "get_messages",
        crate::domain::ids::AgentId::from("dead-child"),
        None,
        Some(crate::domain::ids::MessageId::from("missing-cursor")),
    )
    .await;
    let json = serde_json::to_value(ev).unwrap();

    assert_eq!(json["success"], false);
    assert_eq!(json["error"], "history cursor not found: missing-cursor");
}
