use super::*;
use tempfile::TempDir;

fn chat_message(role: Role, content: &str) -> Message {
    match role {
        Role::System => Message::system(content),
        Role::User => Message::user(content),
        Role::Assistant => Message::assistant(content, vec![]),
        Role::Tool => Message::tool("call", content),
    }
}

#[tokio::test]
async fn list_returns_only_user_chat_sessions_with_metadata_newest_first() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());

    for key in [
        "cli_subagent",
        "cli_agent_manager_x",
        "cli_quecto_command_agent-x",
        "default",
    ] {
        store
            .save(&Session {
                key: key.to_string(),
                messages: vec![chat_message(Role::User, "internal")],
                workflow_run: None,
            })
            .await
            .unwrap();
    }

    store
        .save(&Session {
            key: "chat-old".to_string(),
            messages: vec![
                chat_message(Role::User, "older chat title"),
                chat_message(Role::System, "sys"),
            ],
            workflow_run: None,
        })
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    store
        .save(&Session {
            key: "chat-new".to_string(),
            messages: vec![
                chat_message(Role::System, "sys"),
                chat_message(Role::User, "newer chat title"),
                chat_message(Role::Assistant, "reply"),
                chat_message(Role::Tool, "tool output"),
            ],
            workflow_run: None,
        })
        .await
        .unwrap();

    let summaries = store
        .list(Some(crate::domain::session::USER_CHAT_PREFIX))
        .await
        .unwrap();

    assert_eq!(summaries.len(), 2);
    assert_eq!(summaries[0].key, "chat-new");
    assert_eq!(summaries[0].title, "newer chat title");
    assert_eq!(summaries[0].message_count, 2);
    assert!(summaries[0].updated_unix_secs.is_some());
    assert_eq!(summaries[1].key, "chat-old");
    assert_eq!(summaries[1].title, "older chat title");
    assert_eq!(summaries[1].message_count, 1);
}

#[tokio::test]
async fn list_extracts_raw_first_user_message() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());
    let long = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let huge = "x".repeat(500);

    store
        .save(&Session {
            key: "chat-long".to_string(),
            messages: vec![chat_message(Role::User, long)],
            workflow_run: None,
        })
        .await
        .unwrap();
    store
        .save(&Session {
            key: "chat-empty".to_string(),
            messages: vec![chat_message(Role::Assistant, "hello")],
            workflow_run: None,
        })
        .await
        .unwrap();
    store
        .save(&Session {
            key: "chat-huge".to_string(),
            messages: vec![chat_message(Role::User, &huge)],
            workflow_run: None,
        })
        .await
        .unwrap();

    let summaries = store
        .list(Some(crate::domain::session::USER_CHAT_PREFIX))
        .await
        .unwrap();
    let long_summary = summaries.iter().find(|s| s.key == "chat-long").unwrap();
    let empty_summary = summaries.iter().find(|s| s.key == "chat-empty").unwrap();
    let huge_summary = summaries.iter().find(|s| s.key == "chat-huge").unwrap();

    // Persistence returns the RAW first user message — no truncation/ellipsis;
    // display formatting (truncation, "(untitled)") lives in the interface layer.
    assert_eq!(long_summary.title, long);
    // No user message → empty title (display applies the "(untitled)" placeholder).
    assert_eq!(empty_summary.title, "");
    // Bounded to the transport cap (200 chars) to keep wire payloads sane.
    assert_eq!(huge_summary.title.chars().count(), 200);
}
