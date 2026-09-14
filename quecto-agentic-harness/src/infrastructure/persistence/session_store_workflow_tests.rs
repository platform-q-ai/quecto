//! Workflow-run persistence round trips of the file session store: the
//! persisted run survives save/load, delta saves and clears (split from the
//! metadata tests by cohesive role).
use super::metadata_tests::{id, make_message};
use super::*;
use crate::infrastructure::persistence::session_layout::FlatSessionLayout;
use tempfile::TempDir;

fn persisted_workflow_run() -> WorkflowRunPersisted {
    WorkflowRunPersisted {
        template_id: Some("fix".to_string()),
        done: vec![true, true, false, false, false, false],
        active_issue: Some((42, "login bug".to_string())),
    }
}

#[tokio::test]
async fn test_workflow_run_survives_round_trip() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(FlatSessionLayout::new(tmp.path()));

    let session = Session {
        key: id("test:wf_persist"),
        messages: vec![make_message(Role::User, "hello")],
        workflow_run: Some(persisted_workflow_run()),
        subagent_roster: Vec::new(),
    };
    store.save(&session).await.unwrap();
    let loaded = store.load(&id("test:wf_persist")).await.unwrap().unwrap();
    let wf = loaded
        .workflow_run
        .expect("workflow_run should survive save/load");
    assert_eq!(wf.template_id.as_deref(), Some("fix"));
    assert_eq!(wf.done, vec![true, true, false, false, false, false]);
    assert_eq!(wf.active_issue, Some((42, "login bug".to_string())));
}

#[tokio::test]
async fn workflow_only_session_survives_round_trip() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(FlatSessionLayout::new(tmp.path()));

    store
        .save(&Session {
            key: id("test:wf_only"),
            messages: Vec::new(),
            workflow_run: Some(persisted_workflow_run()),
            subagent_roster: Vec::new(),
        })
        .await
        .unwrap();

    let loaded = store.load(&id("test:wf_only")).await.unwrap().unwrap();
    assert!(
        loaded.messages.is_empty(),
        "workflow-only sessions must not invent chat messages"
    );
    assert_eq!(
        loaded
            .workflow_run
            .expect("workflow_run should persist")
            .done,
        vec![true, true, false, false, false, false]
    );
}

#[tokio::test]
async fn workflow_only_delta_survives_round_trip() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(FlatSessionLayout::new(tmp.path()));

    store
        .save_delta(
            &id("test:wf_only_delta"),
            &[],
            0,
            Some(persisted_workflow_run()),
        )
        .await
        .unwrap();
    let loaded_delta = store
        .load(&id("test:wf_only_delta"))
        .await
        .unwrap()
        .expect("workflow-only delta should persist");
    assert_eq!(
        loaded_delta
            .workflow_run
            .expect("workflow_run should persist from delta")
            .active_issue,
        Some((42, "login bug".to_string()))
    );

    store
        .save_clean_delta(
            &id("test:wf_only_clean_delta"),
            &[],
            0,
            Some(persisted_workflow_run()),
        )
        .await
        .unwrap();
    let loaded_clean = store
        .load(&id("test:wf_only_clean_delta"))
        .await
        .unwrap()
        .expect("workflow-only clean delta should persist");
    assert_eq!(
        loaded_clean
            .workflow_run
            .expect("workflow_run should persist from clean delta")
            .template_id
            .as_deref(),
        Some("fix")
    );
}

#[tokio::test]
async fn test_workflow_run_none_survives_round_trip() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(FlatSessionLayout::new(tmp.path()));

    let session = Session {
        key: id("test:wf_none"),
        messages: vec![make_message(Role::User, "hello")],
        workflow_run: None,
        subagent_roster: Vec::new(),
    };
    store.save(&session).await.unwrap();
    let loaded = store.load(&id("test:wf_none")).await.unwrap().unwrap();
    assert!(loaded.workflow_run.is_none());
}

#[tokio::test]
async fn appended_delta_can_clear_previous_workflow_run() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(FlatSessionLayout::new(tmp.path()));

    let mut session = Session {
        key: id("test:wf_clear"),
        messages: vec![make_message(Role::User, "hello")],
        workflow_run: Some(WorkflowRunPersisted {
            template_id: Some("fix".to_string()),
            done: vec![true, false],
            active_issue: Some((987, "session persistence".to_string())),
        }),
        subagent_roster: Vec::new(),
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

    let loaded = store.load(&id("test:wf_clear")).await.unwrap().unwrap();
    assert!(loaded.workflow_run.is_none());
}

#[tokio::test]
async fn test_workflow_run_unknown_template_persists_raw_fields() {
    let tmp = TempDir::new().unwrap();
    let store = FileSessionStore::new(FlatSessionLayout::new(tmp.path()));

    let session = Session {
        key: id("test:wf_compat"),
        messages: vec![make_message(Role::User, "hello")],
        workflow_run: Some(WorkflowRunPersisted {
            template_id: Some("deleted_template".to_string()),
            done: vec![true, false],
            active_issue: None,
        }),
        subagent_roster: Vec::new(),
    };
    store.save(&session).await.unwrap();

    let loaded = store.load(&id("test:wf_compat")).await.unwrap().unwrap();
    let wf = loaded.workflow_run.expect("persisted run should load");
    assert_eq!(wf.template_id.as_deref(), Some("deleted_template"));
    assert_eq!(wf.done, vec![true, false]);
}
