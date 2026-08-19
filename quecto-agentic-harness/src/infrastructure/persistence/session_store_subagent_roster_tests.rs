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

fn roster_entry(
    id: &str,
    liveness: crate::domain::session::SubagentLiveness,
) -> crate::domain::session::PersistedSubagentRosterEntry {
    crate::domain::session::PersistedSubagentRosterEntry {
        agent_uuid: id.to_string(),
        display_name: format!("worker-{id}"),
        session_key: format!("cli:{id}"),
        socket_path: format!("/tmp/{id}.sock").into(),
        pid: 100,
        liveness,
        parent_id: Some("root".to_string()),
        read_only: id == "dead",
        delivered_message_ordinal: None,
        status: Some(
            match liveness {
                crate::domain::session::SubagentLiveness::Live => "idle",
                crate::domain::session::SubagentLiveness::Detached => "exited",
                crate::domain::session::SubagentLiveness::Dead => "exited",
            }
            .to_string(),
        ),
    }
}

#[tokio::test]
async fn subagent_roster_roundtrips_and_legacy_files_load_empty_roster() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());
    let session = Session {
        key: "cli:roster".to_string(),
        messages: vec![make_message(Role::User, "hello")],
        workflow_run: None,
        subagent_roster: vec![
            roster_entry("live", crate::domain::session::SubagentLiveness::Live),
            roster_entry(
                "detached",
                crate::domain::session::SubagentLiveness::Detached,
            ),
            roster_entry("dead", crate::domain::session::SubagentLiveness::Dead),
        ],
    };

    store.save(&session).await.unwrap();
    let loaded = store.load("cli:roster").await.unwrap().unwrap();
    assert_eq!(loaded.subagent_roster, session.subagent_roster);

    let legacy_path = store.session_path("cli:legacy");
    store.ensure_dir().await.unwrap();
    tokio::fs::write(
        legacy_path,
        r#"{"key":"cli:legacy","messages":[{"role":"user","content":"old"}]}"#,
    )
    .await
    .unwrap();
    let legacy = store.load("cli:legacy").await.unwrap().unwrap();
    assert!(legacy.subagent_roster.is_empty());
}

#[tokio::test]
async fn roster_only_session_persists_and_empty_roster_session_stays_absent() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());

    store
        .save(&Session {
            key: "cli:roster-only".to_string(),
            messages: vec![],
            workflow_run: None,
            subagent_roster: vec![roster_entry(
                "dead",
                crate::domain::session::SubagentLiveness::Dead,
            )],
        })
        .await
        .unwrap();
    assert!(store.exists("cli:roster-only").await.unwrap());
    assert_eq!(
        store
            .load("cli:roster-only")
            .await
            .unwrap()
            .unwrap()
            .subagent_roster
            .len(),
        1
    );

    store
        .save(&Session {
            key: "cli:empty".to_string(),
            messages: vec![],
            workflow_run: None,
            subagent_roster: vec![],
        })
        .await
        .unwrap();
    assert!(!store.exists("cli:empty").await.unwrap());
}

#[tokio::test]
async fn roster_only_updates_replay_as_full_replacements() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());
    let mut session = Session {
        key: "cli:roster-delta".to_string(),
        messages: vec![make_message(Role::User, "hello")],
        workflow_run: None,
        subagent_roster: vec![roster_entry(
            "a",
            crate::domain::session::SubagentLiveness::Live,
        )],
    };
    store.save(&session).await.unwrap();

    session.subagent_roster = vec![roster_entry(
        "a",
        crate::domain::session::SubagentLiveness::Dead,
    )];
    store.save(&session).await.unwrap();
    session.subagent_roster = vec![
        roster_entry("a", crate::domain::session::SubagentLiveness::Dead),
        roster_entry("b", crate::domain::session::SubagentLiveness::Detached),
    ];
    store.save(&session).await.unwrap();

    let loaded = store.load("cli:roster-delta").await.unwrap().unwrap();
    assert_eq!(loaded.messages.len(), 1);
    assert_eq!(loaded.subagent_roster, session.subagent_roster);
}

#[tokio::test]
async fn compaction_retains_current_subagent_roster() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());
    let mut session = Session {
        key: "cli:roster-compact".to_string(),
        messages: vec![make_message(Role::User, "first")],
        workflow_run: None,
        subagent_roster: vec![roster_entry(
            "a",
            crate::domain::session::SubagentLiveness::Live,
        )],
    };
    store.save(&session).await.unwrap();
    session.messages = vec![make_message(Role::User, "replacement")];
    session.subagent_roster = vec![roster_entry(
        "a",
        crate::domain::session::SubagentLiveness::Dead,
    )];
    store.save(&session).await.unwrap();

    let loaded = store.load("cli:roster-compact").await.unwrap().unwrap();
    assert_eq!(loaded.messages[0].content, "replacement");
    assert_eq!(loaded.subagent_roster, session.subagent_roster);
}

#[tokio::test]
async fn save_delta_compaction_preserves_persisted_subagent_roster() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());
    let session = Session {
        key: "cli:roster-delta-compact".to_string(),
        messages: vec![make_message(Role::User, "first")],
        workflow_run: None,
        subagent_roster: vec![roster_entry(
            "a",
            crate::domain::session::SubagentLiveness::Detached,
        )],
    };
    store.save(&session).await.unwrap();

    store
        .save_delta(
            "cli:roster-delta-compact",
            &[make_message(Role::User, "replacement")],
            0,
            None,
        )
        .await
        .unwrap();

    let loaded = store
        .load("cli:roster-delta-compact")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loaded.messages[0].content, "replacement");
    assert_eq!(loaded.subagent_roster, session.subagent_roster);
}
