//! RED persistence contracts for #1612 execution scope metadata.
//!
//! The assertions are semantic except for the required dedicated
//! metadata-only JSONL record: changing activation metadata must not masquerade
//! as a message append and must not move transcript offsets.

use super::*;
use crate::domain::session::{
    AgentDisplayName, ExecutionMetadata, ExecutionMetadataWrite, FolderDisplayLabel,
    FolderIdentity, GitBranchDisplay, PersistedSubagentRosterEntry, SubagentLiveness,
    SubagentRestoreReason,
};
use crate::domain::workflow::WorkflowRunPersisted;
use std::path::PathBuf;
use tempfile::TempDir;

fn metadata(folder: &str, label: &str, agent: &str, branch: Option<&str>) -> ExecutionMetadata {
    ExecutionMetadata::new(
        FolderIdentity::from_unix_bytes(folder.as_bytes().to_vec()),
        FolderDisplayLabel::new(label),
        AgentDisplayName::new(agent),
        branch.and_then(GitBranchDisplay::new),
    )
}

fn message(content: &str, ordinal: u64) -> Message {
    let mut message = Message::user(content);
    message.ordinal = Some(ordinal);
    message
}

fn session_with_metadata(
    key: &str,
    origin: &ExecutionMetadata,
    latest: &ExecutionMetadata,
    messages: Vec<Message>,
) -> Session {
    let mut session = Session::new(key);
    session.messages = messages;
    session.initialize_execution_metadata(origin.clone());
    session.update_latest_execution_metadata(latest.clone());
    session
}

fn assert_execution_metadata(
    session: &Session,
    origin: Option<&ExecutionMetadata>,
    latest: Option<&ExecutionMetadata>,
) {
    assert_eq!(session.origin_execution_metadata(), origin);
    assert_eq!(session.latest_execution_metadata(), latest);
}

fn assert_contents_and_ordinals(session: &Session, expected: &[(&str, u64)]) {
    let actual = session
        .messages
        .iter()
        .map(|message| (message.content.as_str(), message.ordinal.unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}

#[tokio::test]
async fn full_and_ordinary_saves_round_trip_both_metadata_snapshots() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());
    let origin = metadata("/work/a", "/work/a", "builder", Some("main"));
    let latest = metadata("/work/b", "/work/b", "reviewer", Some("review"));
    let session = session_with_metadata(
        "cli:1612-full",
        &origin,
        &latest,
        vec![message("one", 10), message("two", 11)],
    );

    store.save(&session).await.unwrap();
    let fresh_store = FileSessionStore::new(tmp.path());
    let mut loaded = fresh_store.load(&session.key).await.unwrap().unwrap();
    assert_execution_metadata(&loaded, Some(&origin), Some(&latest));
    assert_contents_and_ordinals(&loaded, &[("one", 10), ("two", 11)]);

    loaded.messages.push(message("ordinary append", 12));
    store.save(&loaded).await.unwrap();
    let reloaded = FileSessionStore::new(tmp.path())
        .load(&session.key)
        .await
        .unwrap()
        .unwrap();
    assert_execution_metadata(&reloaded, Some(&origin), Some(&latest));
    assert_contents_and_ordinals(
        &reloaded,
        &[("one", 10), ("two", 11), ("ordinary append", 12)],
    );

    let metadata_unaware = Session::from_parts(
        session.key.clone(),
        vec![message("production full save", 13)],
        None,
        Vec::new(),
    );
    store.save(&metadata_unaware).await.unwrap();
    let reloaded = store.load(&session.key).await.unwrap().unwrap();
    assert_execution_metadata(&reloaded, Some(&origin), Some(&latest));
    assert_contents_and_ordinals(&reloaded, &[("production full save", 13)]);
}

#[tokio::test]
async fn legacy_snapshot_and_jsonl_load_without_inventing_execution_metadata() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());
    store.ensure_dir().await.unwrap();

    tokio::fs::write(
        store.session_path("cli:legacy-single"),
        r#"{"key":"cli:legacy-single","messages":[{"ordinal":7,"role":"user","content":"old"}],"futureTopLevel":{"ignored":true}}"#,
    )
    .await
    .unwrap();
    tokio::fs::write(
        store.session_path("cli:legacy-jsonl"),
        concat!(
            r#"{"type":"snapshot","key":"cli:legacy-jsonl","messages":[{"ordinal":8,"role":"user","content":"first"}],"futureSnapshotField":17}"#,
            "\n",
            r#"{"type":"append","start_index":1,"messages":[{"ordinal":9,"role":"assistant","content":"second"}],"futureAppendField":"ignored"}"#,
            "\n"
        ),
    )
    .await
    .unwrap();

    let single = store.load("cli:legacy-single").await.unwrap().unwrap();
    assert_execution_metadata(&single, None, None);
    assert_contents_and_ordinals(&single, &[("old", 7)]);

    let jsonl = store.load("cli:legacy-jsonl").await.unwrap().unwrap();
    assert_execution_metadata(&jsonl, None, None);
    assert_contents_and_ordinals(&jsonl, &[("first", 8), ("second", 9)]);
}

#[tokio::test]
async fn metadata_only_record_replaces_latest_without_changing_origin_or_offsets() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());
    let origin = metadata("/work/a", "/work/a", "builder", Some("main"));
    let previous_latest = metadata("/work/b", "/work/b", "reviewer", Some("review"));
    let detached_latest = metadata("/work/c", "/work/c", "operator", None);
    let mut session = session_with_metadata(
        "cli:1612-metadata-only",
        &origin,
        &previous_latest,
        vec![message("one", 20), message("two", 30)],
    );
    session.workflow_run = Some(WorkflowRunPersisted {
        template_id: Some("feature".to_string()),
        done: vec![true, false],
        active_issue: Some((1612, "folder resume".to_string())),
    });
    session.subagent_roster = vec![PersistedSubagentRosterEntry {
        agent_uuid: "00000000-0000-0000-0000-000000001612".to_string(),
        display_name: "worker".to_string(),
        session_key: "cli:worker".to_string(),
        socket_path: PathBuf::from("/tmp/worker.sock"),
        pid: 1612,
        liveness: SubagentLiveness::Live,
        restore_reason: SubagentRestoreReason::LegacyUnspecified,
        parent_id: None,
        read_only: false,
        status: Some("working".to_string()),
        delivered_message_ordinal: Some(30),
        pending_message_reports: Default::default(),
    }];
    let expected_workflow = session.workflow_run.clone();
    let expected_roster = session.subagent_roster.clone();
    store.save(&session).await.unwrap();
    let path = store.session_path(&session.key);
    let before = tokio::fs::read(&path).await.unwrap();

    <FileSessionStore as SessionStore>::save_execution_metadata(
        &store,
        &session.key,
        ExecutionMetadataWrite::UpdateLatest(detached_latest.clone()),
    )
    .await
    .unwrap();

    let after = tokio::fs::read(&path).await.unwrap();
    assert!(after.starts_with(&before));
    let appended: serde_json::Value =
        serde_json::from_slice(after[before.len()..].strip_suffix(b"\n").unwrap()).unwrap();
    assert_eq!(appended["type"], "metadata");
    assert!(appended.get("messages").is_none());
    assert!(appended.get("start_index").is_none());
    assert!(
        appended.get("origin_execution_metadata").is_none(),
        "the latest-only operation must be structurally unable to replace origin"
    );

    let loaded = store.load(&session.key).await.unwrap().unwrap();
    assert_execution_metadata(&loaded, Some(&origin), Some(&detached_latest));
    assert_contents_and_ordinals(&loaded, &[("one", 20), ("two", 30)]);
    assert_eq!(loaded.workflow_run, expected_workflow);
    assert_eq!(loaded.subagent_roster, expected_roster);

    <FileSessionStore as SessionStore>::save_execution_metadata(
        &store,
        &session.key,
        ExecutionMetadataWrite::UpdateLatest(detached_latest.clone()),
    )
    .await
    .unwrap();
    let repeated = store.load(&session.key).await.unwrap().unwrap();
    assert_execution_metadata(&repeated, Some(&origin), Some(&detached_latest));
    assert_contents_and_ordinals(&repeated, &[("one", 20), ("two", 30)]);
    assert_eq!(repeated.workflow_run, expected_workflow);
    assert_eq!(repeated.subagent_roster, expected_roster);
}

#[tokio::test]
async fn malformed_optional_metadata_and_unknown_fields_do_not_poison_valid_siblings() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());
    let origin = metadata("/work/a", "/work/a", "builder", Some("main"));
    let latest = metadata("/work/b", "/work/b", "reviewer", Some("review"));
    let key = "cli:1612-lossy-load";
    store
        .save(&session_with_metadata(
            key,
            &origin,
            &origin,
            vec![message("one", 35)],
        ))
        .await
        .unwrap();
    <FileSessionStore as SessionStore>::save_execution_metadata(
        &store,
        key,
        ExecutionMetadataWrite::UpdateLatest(latest.clone()),
    )
    .await
    .unwrap();

    let path = store.session_path(key);
    let text = tokio::fs::read_to_string(&path).await.unwrap();
    let mut records = text
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    let metadata_record = records.last_mut().unwrap();
    metadata_record["futureRecordField"] = serde_json::json!({"ignored": true});
    let latest_json = metadata_record["latest_execution_metadata"]
        .as_object_mut()
        .expect("metadata-only record carries a latest aggregate");
    latest_json.insert("futureNestedField".to_string(), serde_json::json!(17));
    latest_json.insert(
        "agent_name".to_string(),
        serde_json::json!("x".repeat(AgentDisplayName::MAX_BYTES + 1)),
    );
    let rewritten = records
        .iter()
        .map(|record| serde_json::to_string(record).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    tokio::fs::write(path, rewritten).await.unwrap();

    let loaded = store.load(key).await.unwrap().unwrap();
    assert_eq!(loaded.origin_execution_metadata(), Some(&origin));
    let loaded_latest = loaded.latest_execution_metadata().unwrap();
    assert_eq!(loaded_latest.folder_identity(), latest.folder_identity());
    assert_eq!(loaded_latest.folder_label(), latest.folder_label());
    assert_eq!(loaded_latest.agent_name(), None);
    assert_eq!(loaded_latest.git_branch(), latest.git_branch());
}

#[tokio::test]
async fn metadata_only_update_of_legacy_file_migrates_and_survives_later_saves() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());
    store.ensure_dir().await.unwrap();
    let key = "cli:1612-legacy-update";
    tokio::fs::write(
        store.session_path(key),
        r#"{"key":"cli:1612-legacy-update","messages":[{"ordinal":41,"role":"user","content":"legacy"}]}"#,
    )
    .await
    .unwrap();
    let latest = metadata("/work/new", "/work/new", "resumer", Some("new-branch"));

    <FileSessionStore as SessionStore>::save_execution_metadata(
        &store,
        key,
        ExecutionMetadataWrite::UpdateLatest(latest.clone()),
    )
    .await
    .unwrap();
    let mut loaded = store.load(key).await.unwrap().unwrap();
    assert_execution_metadata(&loaded, Some(&latest), Some(&latest));
    assert_contents_and_ordinals(&loaded, &[("legacy", 41)]);

    loaded.messages.push(message("after resume", 42));
    store.save(&loaded).await.unwrap();
    loaded.messages = vec![message("rewound", 50)];
    store.save(&loaded).await.unwrap();

    let reloaded = store.load(key).await.unwrap().unwrap();
    assert_execution_metadata(&reloaded, Some(&latest), Some(&latest));
    assert_contents_and_ordinals(&reloaded, &[("rewound", 50)]);
    for line in tokio::fs::read_to_string(store.session_path(key))
        .await
        .unwrap()
        .lines()
    {
        serde_json::from_str::<serde_json::Value>(line)
            .expect("legacy migration must leave a valid JSONL record family");
    }
}

#[tokio::test]
async fn append_delta_clean_delta_and_their_compactions_preserve_metadata() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());
    let origin = metadata("/work/a", "/work/a", "builder", Some("main"));
    let latest = metadata("/work/b", "/work/b", "reviewer", Some("review"));
    let key = "cli:1612-deltas";
    let initial = session_with_metadata(
        key,
        &origin,
        &latest,
        vec![message("one", 61), message("two", 62)],
    );
    store.save(&initial).await.unwrap();

    let after_delta = vec![message("one", 61), message("two", 62), message("delta", 63)];
    store.save_delta(key, &after_delta, 2, None).await.unwrap();
    let after_clean = [after_delta.clone(), vec![message("clean delta", 64)]].concat();
    store
        .save_clean_delta(key, &after_clean, 3, None)
        .await
        .unwrap();
    let appended = store.load(key).await.unwrap().unwrap();
    assert_execution_metadata(&appended, Some(&origin), Some(&latest));
    assert_contents_and_ordinals(
        &appended,
        &[("one", 61), ("two", 62), ("delta", 63), ("clean delta", 64)],
    );

    // A changed persisted prefix forces save-delta compaction.
    let changed_prefix = vec![message("changed", 61), message("two", 62)];
    store
        .save_delta(key, &changed_prefix, 2, None)
        .await
        .unwrap();
    let prefix_compacted = store.load(key).await.unwrap().unwrap();
    assert_execution_metadata(&prefix_compacted, Some(&origin), Some(&latest));
    assert_contents_and_ordinals(&prefix_compacted, &[("changed", 61), ("two", 62)]);

    // A shortened history forces the save-delta compaction/rewind branch.
    let rewound = vec![message("changed", 61)];
    store.save_delta(key, &rewound, 2, None).await.unwrap();
    let compacted = store.load(key).await.unwrap().unwrap();
    assert_execution_metadata(&compacted, Some(&origin), Some(&latest));
    assert_contents_and_ordinals(&compacted, &[("changed", 61)]);

    // A zero watermark forces the clean-delta snapshot branch.
    let replaced = vec![message("replacement", 70)];
    store
        .save_clean_delta(key, &replaced, 0, None)
        .await
        .unwrap();
    let clean_compacted = store.load(key).await.unwrap().unwrap();
    assert_execution_metadata(&clean_compacted, Some(&origin), Some(&latest));
    assert_contents_and_ordinals(&clean_compacted, &[("replacement", 70)]);
}

#[tokio::test]
async fn empty_delta_paths_preserve_metadata_only_sessions() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());
    let latest = metadata("/work/a", "/work/a", "builder", Some("main"));
    let key = "cli:1612-empty-delta";
    <FileSessionStore as SessionStore>::save_execution_metadata(
        &store,
        key,
        ExecutionMetadataWrite::UpdateLatest(latest.clone()),
    )
    .await
    .unwrap();

    store.save_delta(key, &[], 0, None).await.unwrap();
    let after_delta = store.load(key).await.unwrap().unwrap();
    assert_execution_metadata(&after_delta, Some(&latest), Some(&latest));
    assert!(after_delta.messages.is_empty());

    <FileSessionStore as SessionStore>::save_clean_delta(&store, key, &[], 0, None)
        .await
        .unwrap();
    let after_clean = store.load(key).await.unwrap().unwrap();
    assert_execution_metadata(&after_clean, Some(&latest), Some(&latest));
    assert!(after_clean.messages.is_empty());
}

#[tokio::test]
async fn session_family_fold_and_summary_use_latest_metadata_only() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());
    let origin = metadata("/work/a", "same lossy label", "builder", Some("main"));
    let latest = metadata("/work/b", "same lossy label", "reviewer", Some("review"));
    let key = "cli:1612-family";
    let session = session_with_metadata(key, &origin, &origin, vec![message("title", 80)]);
    store.save(&session).await.unwrap();
    <FileSessionStore as SessionStore>::save_execution_metadata(
        &store,
        key,
        ExecutionMetadataWrite::UpdateLatest(latest.clone()),
    )
    .await
    .unwrap();
    let mut loaded = store.load(key).await.unwrap().unwrap();
    loaded.messages.push(message("tail", 81));
    store.save(&loaded).await.unwrap();

    let summaries = store.list(Some("cli:1612-family")).await.unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].key, key);
    assert_eq!(
        summaries[0].latest_execution_metadata(),
        Some(&latest),
        "listing must fold the latest record, never origin or a colliding label"
    );

    let unknown_latest = ExecutionMetadata::new(
        None,
        latest.folder_label().cloned(),
        latest.agent_name().cloned(),
        latest.git_branch().cloned(),
    );
    <FileSessionStore as SessionStore>::save_execution_metadata(
        &store,
        key,
        ExecutionMetadataWrite::UpdateLatest(unknown_latest),
    )
    .await
    .unwrap();
    let summaries = store.list(Some("cli:1612-family")).await.unwrap();
    assert_eq!(
        summaries.len(),
        1,
        "unknown scope is still directly listable"
    );
    assert_eq!(
        summaries[0]
            .latest_execution_metadata()
            .unwrap()
            .folder_identity(),
        None,
        "a label must not resurrect the latest folder or fall back to origin"
    );
}
