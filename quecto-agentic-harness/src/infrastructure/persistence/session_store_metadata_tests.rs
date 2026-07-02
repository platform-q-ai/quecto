use super::*;
use tempfile::TempDir;

fn make_message(role: Role, content: &str) -> Message {
    match role {
        Role::System => Message::system(content),
        Role::User => Message::user(content),
        Role::Assistant => Message::assistant(content, vec![]),
        Role::Tool => Message::tool("call", content),
    }
}

#[tokio::test]
async fn test_save_and_load_session() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());

    let session = Session {
        key: "telegram:12345".to_string(),
        messages: vec![
            make_message(Role::User, "Hello"),
            make_message(Role::Assistant, "Hi there!"),
        ],
        workflow_run: None,
    };

    store.save(&session).await.unwrap();
    let loaded = store.load("telegram:12345").await.unwrap();
    assert!(loaded.is_some());
    let loaded = loaded.unwrap();
    assert_eq!(loaded.key, "telegram:12345");
    assert_eq!(loaded.messages.len(), 2);
    assert_eq!(loaded.messages[0].content, "Hello");
    assert_eq!(loaded.messages[0].role, Role::User);
    assert_eq!(loaded.messages[1].content, "Hi there!");
    assert_eq!(loaded.messages[1].role, Role::Assistant);
}

#[tokio::test]
async fn test_load_nonexistent_session() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());

    let loaded = store.load("nonexistent").await.unwrap();
    assert!(loaded.is_none());
}

#[tokio::test]
async fn test_exists() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());

    assert!(!store.exists("telegram:12345").await.unwrap());

    let session = Session::new("telegram:12345");
    store.save(&session).await.unwrap();

    assert!(store.exists("telegram:12345").await.unwrap());
}

#[tokio::test]
async fn test_key_to_filename() {
    assert_eq!(
        FileSessionStore::key_to_filename("telegram:12345"),
        "telegram_12345.json"
    );
    assert_eq!(
        FileSessionStore::key_to_filename("cli:default"),
        "cli_default.json"
    );
}

#[test]
fn test_key_to_filename_sanitizes_path_traversal_chars() {
    let filename = FileSessionStore::key_to_filename("../../tmp/escape");
    assert!(!filename.contains(".."));
    assert!(!filename.contains('/'));
    assert!(!filename.contains('\\'));
    assert!(filename.ends_with(".json"));
}

#[test]
fn test_key_to_filename_avoids_collision_for_unsafe_keys() {
    let a = FileSessionStore::key_to_filename("a/b");
    let b = FileSessionStore::key_to_filename("a?b");
    assert_ne!(a, b);
}

#[tokio::test]
async fn test_session_with_tool_calls() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());

    let session = Session {
        key: "test:tools".to_string(),
        messages: vec![
            make_message(Role::User, "run a command"),
            Message::assistant(
                String::new(),
                vec![ToolCall {
                    id: "call_1".to_string(),
                    name: "bash".to_string(),
                    arguments: r#"{"command":"ls"}"#.to_string(),
                }],
            ),
            Message::tool("call_1", "file1.txt\nfile2.txt"),
        ],
        workflow_run: None,
    };

    store.save(&session).await.unwrap();
    let loaded = store.load("test:tools").await.unwrap().unwrap();
    assert_eq!(loaded.messages.len(), 3);
    assert_eq!(loaded.messages[1].tool_calls.len(), 1);
    assert_eq!(loaded.messages[1].tool_calls[0].name, "bash");
    assert_eq!(loaded.messages[2].tool_call_id.as_deref(), Some("call_1"));
}

#[test]
fn test_session_build_key() {
    assert_eq!(Session::build_key("telegram", "12345"), "telegram:12345");
    assert_eq!(Session::build_key("cli", "default"), "cli:default");
}

#[tokio::test]
async fn test_persistence_across_store_instances() {
    let tmp = TempDir::new().unwrap();

    // Save with one store instance
    let store1 = FileSessionStore::new(tmp.path());
    let session = Session {
        key: "telegram:persist".to_string(),
        messages: vec![make_message(Role::User, "persisted message")],
        workflow_run: None,
    };
    store1.save(&session).await.unwrap();

    // Load with a new store instance pointing to the same directory
    let store2 = FileSessionStore::new(tmp.path());
    let loaded = store2.load("telegram:persist").await.unwrap();
    assert!(loaded.is_some());
    assert_eq!(loaded.unwrap().messages[0].content, "persisted message");
}

// --- Pruning metadata round-trip tests ---

#[tokio::test]
async fn test_turn_field_survives_round_trip() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());

    let mut tool_msg = Message::tool("call_1", "tool output");
    tool_msg.turn = Some(3);

    let session = Session {
        key: "test:turn".to_string(),
        messages: vec![tool_msg],
        workflow_run: None,
    };
    store.save(&session).await.unwrap();
    let loaded = store.load("test:turn").await.unwrap().unwrap();
    assert_eq!(
        loaded.messages[0].turn,
        Some(3),
        "turn field should survive save/load"
    );
}

#[tokio::test]
async fn test_is_collapsed_survives_round_trip() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());

    let mut tool_msg = Message::tool("call_1", "[bash: echo hello (100 tokens)]");
    tool_msg.is_collapsed = true;

    let session = Session {
        key: "test:collapsed".to_string(),
        messages: vec![tool_msg],
        workflow_run: None,
    };
    store.save(&session).await.unwrap();
    let loaded = store.load("test:collapsed").await.unwrap().unwrap();
    assert!(
        loaded.messages[0].is_collapsed,
        "is_collapsed should survive save/load"
    );
}

#[tokio::test]
async fn test_is_manifest_survives_round_trip() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());

    let mut manifest = Message::system("[Session memory: 5 spilled entries]");
    manifest.is_manifest = true;
    manifest.is_pinned = true;

    let session = Session {
        key: "test:manifest".to_string(),
        messages: vec![manifest],
        workflow_run: None,
    };
    store.save(&session).await.unwrap();
    let loaded = store.load("test:manifest").await.unwrap().unwrap();
    assert!(
        loaded.messages[0].is_manifest,
        "is_manifest should survive save/load"
    );
}

#[tokio::test]
async fn test_is_pinned_survives_round_trip() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());

    let mut user_msg = Message::user("first message");
    user_msg.is_pinned = true;

    let session = Session {
        key: "test:pinned".to_string(),
        messages: vec![user_msg],
        workflow_run: None,
    };
    store.save(&session).await.unwrap();
    let loaded = store.load("test:pinned").await.unwrap().unwrap();
    assert!(
        loaded.messages[0].is_pinned,
        "is_pinned should survive save/load for non-system messages"
    );
}

#[tokio::test]
async fn test_tool_name_survives_round_trip() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());

    let mut tool_msg = Message::tool("call_1", "output");
    tool_msg.tool_name = Some("bash".to_string());

    let session = Session {
        key: "test:toolname".to_string(),
        messages: vec![tool_msg],
        workflow_run: None,
    };
    store.save(&session).await.unwrap();
    let loaded = store.load("test:toolname").await.unwrap().unwrap();
    assert_eq!(
        loaded.messages[0].tool_name.as_deref(),
        Some("bash"),
        "tool_name should survive save/load"
    );
}

#[tokio::test]
async fn test_input_preview_survives_round_trip() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());

    let mut tool_msg = Message::tool("call_1", "output");
    tool_msg.input_preview = Some("echo hello".to_string());

    let session = Session {
        key: "test:preview".to_string(),
        messages: vec![tool_msg],
        workflow_run: None,
    };
    store.save(&session).await.unwrap();
    let loaded = store.load("test:preview").await.unwrap().unwrap();
    assert_eq!(
        loaded.messages[0].input_preview.as_deref(),
        Some("echo hello"),
        "input_preview should survive save/load"
    );
}

#[tokio::test]
async fn test_spill_id_survives_round_trip() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());

    let mut tool_msg = Message::tool("call_1", "output");
    tool_msg.spill_id = Some("turn1:bash:0".to_string());

    let session = Session {
        key: "test:spillid".to_string(),
        messages: vec![tool_msg],
        workflow_run: None,
    };
    store.save(&session).await.unwrap();
    let loaded = store.load("test:spillid").await.unwrap().unwrap();
    assert_eq!(
        loaded.messages[0].spill_id.as_deref(),
        Some("turn1:bash:0"),
        "spill_id should survive save/load"
    );
}

#[tokio::test]
async fn test_workflow_run_survives_round_trip() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());

    let session = Session {
        key: "test:wf_persist".to_string(),
        messages: vec![make_message(Role::User, "hello")],
        workflow_run: Some(WorkflowRunPersisted {
            template_id: Some("fix".to_string()),
            done: vec![true, true, false, false, false, false],
            active_issue: Some((42, "login bug".to_string())),
        }),
    };
    store.save(&session).await.unwrap();
    let loaded = store.load("test:wf_persist").await.unwrap().unwrap();
    let wf = loaded
        .workflow_run
        .expect("workflow_run should survive save/load");
    assert_eq!(wf.template_id.as_deref(), Some("fix"));
    assert_eq!(wf.done, vec![true, true, false, false, false, false]);
    assert_eq!(wf.active_issue, Some((42, "login bug".to_string())));
}

#[tokio::test]
async fn test_workflow_run_none_survives_round_trip() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());

    let session = Session {
        key: "test:wf_none".to_string(),
        messages: vec![make_message(Role::User, "hello")],
        workflow_run: None,
    };
    store.save(&session).await.unwrap();
    let loaded = store.load("test:wf_none").await.unwrap().unwrap();
    assert!(loaded.workflow_run.is_none());
}

#[tokio::test]
async fn appended_delta_can_clear_previous_workflow_run() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());

    let mut session = Session {
        key: "test:wf_clear".to_string(),
        messages: vec![make_message(Role::User, "hello")],
        workflow_run: Some(WorkflowRunPersisted {
            template_id: Some("fix".to_string()),
            done: vec![true, false],
            active_issue: Some((987, "session persistence".to_string())),
        }),
    };
    store.save(&session).await.unwrap();

    session.workflow_run = None;
    store
        .save_delta(
            &session.key,
            &session.messages,
            session.messages.len(),
            None,
        )
        .await
        .unwrap();

    let loaded = store.load("test:wf_clear").await.unwrap().unwrap();
    assert!(loaded.workflow_run.is_none());
}

#[tokio::test]
async fn test_workflow_run_unknown_template_persists_raw_fields() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());

    let session = Session {
        key: "test:wf_compat".to_string(),
        messages: vec![],
        workflow_run: Some(WorkflowRunPersisted {
            template_id: Some("deleted_template".to_string()),
            done: vec![true, false],
            active_issue: None,
        }),
    };
    store.save(&session).await.unwrap();

    let loaded = store.load("test:wf_compat").await.unwrap().unwrap();
    let wf = loaded.workflow_run.expect("persisted run should load");
    assert_eq!(wf.template_id.as_deref(), Some("deleted_template"));
    assert_eq!(wf.done, vec![true, false]);
}

#[tokio::test]
async fn test_list_sessions_returns_cli_names_and_message_counts() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());

    store
        .save(&Session {
            key: "chat-default".to_string(),
            messages: vec![make_message(Role::User, "hello")],
            workflow_run: None,
        })
        .await
        .unwrap();
    store
        .save(&Session {
            key: "chat-work".to_string(),
            messages: vec![
                make_message(Role::User, "question"),
                make_message(Role::Assistant, "answer"),
            ],
            workflow_run: None,
        })
        .await
        .unwrap();

    let summaries = store.list(None).await.unwrap();
    assert_eq!(summaries.len(), 2);
    let work = summaries.iter().find(|s| s.key == "chat-work").unwrap();
    assert_eq!(work.title, "question");
    assert_eq!(work.title, "question");
    assert_eq!(work.message_count, 2);
    let default = summaries.iter().find(|s| s.key == "chat-default").unwrap();
    assert_eq!(default.title, "hello");
    assert_eq!(default.message_count, 1);
}

#[tokio::test]
async fn test_list_sessions_skips_corrupt_json_files() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());

    store
        .save(&Session {
            key: "chat-good".to_string(),
            messages: vec![make_message(Role::User, "hello")],
            workflow_run: None,
        })
        .await
        .unwrap();
    let sessions_dir = tmp.path().join("sessions");
    tokio::fs::write(sessions_dir.join("cli_bad.json"), "{ invalid json\n}")
        .await
        .unwrap();

    let summaries = store.list(None).await.unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].key, "chat-good");
    assert_eq!(summaries[0].title, "hello");
}

#[tokio::test]
async fn test_system_is_pinned_default_survives_round_trip() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());

    let session = Session {
        key: "test:sys_pinned".to_string(),
        messages: vec![Message::system("system prompt")],
        workflow_run: None,
    };
    store.save(&session).await.unwrap();
    let loaded = store.load("test:sys_pinned").await.unwrap().unwrap();
    // System messages are pinned by default in constructor,
    // so this should pass even without explicit persistence —
    // but user messages marked as pinned would fail.
    assert!(
        loaded.messages[0].is_pinned,
        "system message should remain pinned after round-trip"
    );
}

// ── Coverage: full conversion round-trip + pure mappers ─────────────────────

#[tokio::test]
async fn roundtrip_preserves_roles_toolcalls_stop_reason_and_thinking() {
    use crate::domain::message::{StopReason, ThinkingBlock, ToolCall};
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());

    let mut asst = Message::assistant(
        "answer",
        vec![ToolCall {
            id: "c1".into(),
            name: "search".into(),
            arguments: "{}".into(),
        }],
    );
    asst.stop_reason = Some(StopReason::ToolUse);
    asst.thinking_blocks = vec![
        ThinkingBlock::Normal {
            thinking: "reasoning".into(),
            signature: "sig".into(),
        },
        ThinkingBlock::Redacted {
            data: "redacted".into(),
        },
    ];
    asst.is_pinned = true;
    asst.is_manifest = true;
    asst.is_collapsed = true;

    let session = Session {
        key: "cli:roundtrip".to_string(),
        messages: vec![
            Message::system("sys"),
            Message::user("u"),
            asst,
            Message::tool("c1", "tool result"),
        ],
        workflow_run: None,
    };
    store.save(&session).await.unwrap();
    let loaded = store.load("cli:roundtrip").await.unwrap().unwrap();

    assert_eq!(loaded.messages.len(), 4);
    assert_eq!(loaded.messages[0].role, Role::System);
    assert_eq!(loaded.messages[1].role, Role::User);
    let a = &loaded.messages[2];
    assert_eq!(a.role, Role::Assistant);
    assert_eq!(a.tool_calls.len(), 1);
    assert_eq!(a.tool_calls[0].id, "c1");
    assert_eq!(a.stop_reason, Some(StopReason::ToolUse));
    assert_eq!(a.thinking_blocks.len(), 2);
    assert!(a.is_pinned);
    assert!(a.is_manifest);
    assert!(a.is_collapsed);
    let t = &loaded.messages[3];
    assert_eq!(t.role, Role::Tool);
    assert_eq!(t.tool_call_id.as_deref(), Some("c1"));
}

#[test]
fn stop_reason_to_str_covers_all_variants() {
    use crate::domain::message::StopReason;
    let cases = [
        (StopReason::EndTurn, "end_turn"),
        (StopReason::MaxTokens, "max_tokens"),
        (StopReason::ToolUse, "tool_use"),
        (StopReason::Refusal, "refusal"),
        (StopReason::Error, "error"),
        (StopReason::Aborted, "aborted"),
    ];
    for (sr, s) in cases {
        assert_eq!(sr.to_string(), s);
        assert_eq!(StopReason::parse(s), sr);
    }
    assert_eq!(StopReason::Unknown("weird".into()).to_string(), "weird");
}

#[test]
fn role_str_mapping_roundtrips_and_defaults() {
    for r in [Role::System, Role::User, Role::Assistant, Role::Tool] {
        assert_eq!(super::str_to_role(super::role_to_str(&r)), r);
    }
    assert_eq!(super::str_to_role("not-a-role"), Role::User);
}

// --- #765: list() derives summaries from a lightweight header only ---
//
// Acceptance: list() computes title/count without fully deserializing the
// `messages` array (heavy per-message fields like thinking_blocks). This is
// pinned structurally: the fixture carries a thinking_blocks entry with a tag
// the full MessageRecord/ThinkingBlockRecord model *rejects* (the enum has no
// catch-all). The companion assertion below proves a full parse (load()) does
// reject this exact file, so list() can only succeed by NOT paying the
// full-deserialize cost — a catch-all on ThinkingBlockRecord would break the
// load() assertion and so cannot game this test.
const HEAVY_UNPARSEABLE_SESSION: &str = r#"{
    "key": "chat-heavy",
    "messages": [
        {"role": "user", "content": "what is the answer"},
        {"role": "assistant", "content": "42",
         "thinking_blocks": [{"type": "totally-unknown-variant", "x": 1}]}
    ]
}"#;

#[tokio::test]
async fn test_list_ignores_unparseable_heavy_message_fields() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());

    let sessions_dir = tmp.path().join("sessions");
    tokio::fs::create_dir_all(&sessions_dir).await.unwrap();
    tokio::fs::write(
        sessions_dir.join("chat-heavy.json"),
        HEAVY_UNPARSEABLE_SESSION,
    )
    .await
    .unwrap();

    // A full parse (load) MUST reject the unknown heavy field; otherwise the
    // listing assertion would not prove the header path skips message bodies.
    assert!(
        store.load("chat-heavy").await.is_err(),
        "fixture must be unparseable by the full message model"
    );

    let summaries = store.list(None).await.unwrap();
    let s = summaries
        .iter()
        .find(|s| s.key == "chat-heavy")
        .expect("session with unparseable heavy field should still be listed");
    assert_eq!(s.title, "what is the answer");
    assert_eq!(s.message_count, 2);
}

// Sync guard for the duplicated lightweight schema (#765 review). The header
// is an independent serde view of the full SessionFile/MessageRecord model;
// nothing at the type level links `key`/`messages`/`role`/`content`. This test
// serializes REAL records (exactly what save() writes) and asserts the header
// reads back identical `key`/`role`/`content`. If a field is renamed or its
// representation changes on the authoritative model, the header would fall back
// to its serde defaults and this test fails — instead of silently producing
// blank titles in production.
#[test]
fn test_session_header_stays_in_sync_with_full_record() {
    let file = SessionFile {
        key: "chat-sync".to_string(),
        messages: vec![
            message_to_record(&make_message(Role::User, "first user line")),
            message_to_record(&make_message(Role::Assistant, "assistant reply")),
        ],
        workflow_run: None,
    };
    let json = serde_json::to_string(&file).unwrap();

    let header: SessionHeader = serde_json::from_str(&json).unwrap();
    assert_eq!(header.key, "chat-sync");
    assert_eq!(header.messages.len(), 2);
    assert_eq!(header.messages[0].role, "user");
    assert_eq!(header.messages[0].content, "first user line");
    assert_eq!(header.messages[1].role, "assistant");
    assert_eq!(header.messages[1].content, "assistant reply");
}

// list() is summary-only and is NOT a load guarantee (#765 review): a session
// whose message bodies are malformed is still surfaced by list() but rejected
// by load(). Callers must degrade gracefully. This pins that exact contract.
#[tokio::test]
async fn test_listed_session_with_malformed_body_fails_to_load() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());

    let sessions_dir = tmp.path().join("sessions");
    tokio::fs::create_dir_all(&sessions_dir).await.unwrap();
    tokio::fs::write(
        sessions_dir.join("chat-heavy.json"),
        HEAVY_UNPARSEABLE_SESSION,
    )
    .await
    .unwrap();

    // It is listed (summary derivable from the header)...
    let summaries = store.list(None).await.unwrap();
    assert!(
        summaries.iter().any(|s| s.key == "chat-heavy"),
        "summary-only listing should surface the session"
    );

    // ...but opening it returns an error, NOT a panic or silent success. The
    // caller is expected to handle this gracefully.
    assert!(
        store.load("chat-heavy").await.is_err(),
        "a listed session is not guaranteed to load"
    );
}
