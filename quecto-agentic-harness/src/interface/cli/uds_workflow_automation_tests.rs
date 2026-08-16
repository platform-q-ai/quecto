use super::dispatch_test_env::DispatchTestEnv;
use crate::domain::session::{Session, SessionStore};

fn persisted_feature_run(done: Vec<bool>) -> crate::domain::workflow::WorkflowRunPersisted {
    crate::domain::workflow::WorkflowRunPersisted {
        template_id: Some("feature".into()),
        done,
        active_issue: None,
    }
}

#[tokio::test]
async fn new_session_resets_workflow_run_state() {
    let mut env = DispatchTestEnv::with_selected_feature();
    env.messages = vec![crate::domain::message::Message::user("old")];
    let workflow = env.workflow.clone();
    workflow.lock().unwrap().check(1).unwrap();
    assert!(workflow.lock().unwrap().persisted_run().is_some());
    let mut ctx = env.ctx();

    super::uds_dispatch::handle_new_session(&mut ctx, Some("n"), "new_session").await;

    assert!(workflow.lock().unwrap().persisted_run().is_none());
}

#[tokio::test]
async fn resume_session_restores_target_workflow_run_state() {
    let mut env = DispatchTestEnv::with_unselected_workflow();
    env.messages = vec![crate::domain::message::Message::user("current")];
    let key = Session::build_key("cli", "saved");
    env.store
        .save(&Session {
            key: key.clone(),
            messages: vec![crate::domain::message::Message::user("restored")],
            workflow_run: Some(persisted_feature_run(vec![true, false, false])),
            subagent_roster: Vec::new(),
        })
        .await
        .unwrap();
    let workflow = env.workflow.clone();
    let mut ctx = env.ctx();

    super::uds_dispatch::handle_resume_session(
        &mut ctx,
        Some("r"),
        "resume_session",
        "saved".into(),
    )
    .await;

    let persisted = workflow.lock().unwrap().persisted_run().unwrap();
    assert_eq!(persisted.template_id.as_deref(), Some("feature"));
    assert!(persisted.done[0]);
}

#[tokio::test]
async fn resume_session_clears_workflow_when_target_has_none() {
    let mut env = DispatchTestEnv::with_selected_feature();
    env.messages = vec![crate::domain::message::Message::user("current")];
    env.store
        .save(&Session {
            key: Session::build_key("cli", "plain"),
            messages: vec![crate::domain::message::Message::user("plain")],
            workflow_run: None,
            subagent_roster: Vec::new(),
        })
        .await
        .unwrap();
    let workflow = env.workflow.clone();
    workflow.lock().unwrap().check(1).unwrap();
    let mut ctx = env.ctx();

    super::uds_dispatch::handle_resume_session(
        &mut ctx,
        Some("r"),
        "resume_session",
        "plain".into(),
    )
    .await;

    assert!(workflow.lock().unwrap().persisted_run().is_none());
}

#[tokio::test]
async fn set_workflow_automation_updates_config_and_engine() {
    let mut env = DispatchTestEnv::with_unselected_workflow();
    let workflow = env.workflow.clone();
    let mut ctx = env.ctx();

    super::uds_dispatch::handle_set_workflow_automation(
        &mut ctx,
        Some("wf"),
        "set_workflow_automation",
        Some(false),
        Some(false),
    )
    .await;

    let config = ctx.workflow_config.clone().unwrap();
    assert!(!config.auto_continue);
    assert!(!config.completion_nudge);
    let engine = workflow.lock().unwrap();
    assert!(!engine.auto_continue_enabled());
    assert!(!engine.completion_nudge_enabled());
}

#[test]
fn workflow_nudge_message_waits_for_selected_template() {
    let mut env = DispatchTestEnv::with_unselected_workflow();
    let workflow = env.workflow.clone();
    let ctx = env.ctx();

    assert!(super::workflow_nudge_message(&ctx).is_none());
    workflow
        .lock()
        .unwrap()
        .select_template("feature", None)
        .unwrap();
    let nudge = super::workflow_nudge_message(&ctx).unwrap();
    assert!(nudge.is_auto_continue());
    assert!(nudge.into_message(false).contains("Workflow incomplete"));
}

#[test]
fn workflow_nudge_message_is_suppressed_while_direct_child_active() {
    use crate::infrastructure::tools::subagent_registry::{
        SubagentEntry, SubagentStatus, new_registry,
    };
    let mut env = DispatchTestEnv::with_selected_feature();
    let reg = new_registry();
    {
        let mut guard = reg.lock().unwrap();
        let mut child = SubagentEntry::new("/tmp/child.sock".into(), 1);
        child.status = SubagentStatus::Running;
        guard.insert("child".to_string(), child);
    }
    let mut ctx = env.ctx();
    ctx.subagent_registry = Some(reg);

    assert!(super::workflow_nudge_message(&ctx).is_none());
}

#[test]
fn workflow_nudge_message_is_suppressed_while_transitive_descendant_active() {
    use crate::infrastructure::tools::subagent_registry::{
        SubagentEntry, SubagentStatus, new_registry,
    };
    let mut env = DispatchTestEnv::with_selected_feature();
    let reg = new_registry();
    {
        let mut guard = reg.lock().unwrap();
        let mut child = SubagentEntry::new("/tmp/child.sock".into(), 1);
        child.status = SubagentStatus::Idle;
        guard.insert("child".to_string(), child);
        let mut grandchild = SubagentEntry::new("/tmp/grandchild.sock".into(), 2);
        grandchild.status = SubagentStatus::Starting;
        grandchild.parent_id = Some("child".to_string());
        guard.insert("grandchild".to_string(), grandchild);
    }
    let mut ctx = env.ctx();
    ctx.subagent_registry = Some(reg);

    assert!(super::workflow_nudge_message(&ctx).is_none());
}

#[test]
fn workflow_nudge_message_resumes_after_descendants_stop_being_active() {
    use crate::infrastructure::tools::subagent_registry::{
        SubagentEntry, SubagentStatus, new_registry,
    };
    let mut env = DispatchTestEnv::with_selected_feature();
    let reg = new_registry();
    {
        let mut guard = reg.lock().unwrap();
        let mut child = SubagentEntry::new("/tmp/child.sock".into(), 1);
        child.status = SubagentStatus::Error;
        guard.insert("child".to_string(), child);
        let mut exited = SubagentEntry::new("/tmp/exited.sock".into(), 2);
        exited.status = SubagentStatus::Exited;
        guard.insert("exited".to_string(), exited);
    }
    let mut ctx = env.ctx();
    ctx.subagent_registry = Some(reg);

    assert!(super::workflow_nudge_message(&ctx).is_some());
}

#[tokio::test]
async fn drain_refreshes_busy_state_snapshot_per_turn() {
    // #899: a busy workflow child inspected mid-workflow must see CURRENT state,
    // not the pre-turn/initial snapshot. The snapshots are refreshed after each
    // inner turn inside the drain loop — so message count and workflow advance
    // step-by-step instead of staying frozen until the whole command returns.
    let mut env = DispatchTestEnv::with_selected_feature();
    let mut ctx = env.ctx();
    // The shared state snapshot starts at the pre-turn (initial) view: no
    // workflow attached, zero messages — exactly what a busy child wrongly
    // served before #899.
    let state_snapshot = ctx.state_snapshot.clone();

    // Two pending messages drive TWO inner turns through the drain loop, so the
    // refresh fires per turn across a multi-turn chain (AC2) — not just once when
    // the whole command returns. Each turn adds a user + assistant message, so a
    // step-by-step refresh leaves the snapshot at the full post-chain count.
    ctx.session.enqueue_pending("do the work".to_string());
    ctx.session.enqueue_pending("keep going".to_string());
    super::drain_and_run_pending(&mut ctx).await;

    let expected_count = ctx.messages.len();
    assert!(
        expected_count >= 4,
        "two inner turns should each add a user+assistant message, got {expected_count}"
    );

    // AC1: state snapshot tracks the live workflow + advanced message count.
    let snap = state_snapshot.read().await;
    assert_eq!(
        snap.message_count, expected_count,
        "busy state snapshot must reflect the post-chain message count, not the initial pre-turn view"
    );
    assert!(
        snap.workflow.is_some(),
        "busy state snapshot must reflect the selected workflow, not the initial pre-turn view"
    );
    drop(snap);

    // AC3: the conversation + session_stats snapshots refresh per turn too, not
    // just get_state — a regression dropping any of those refresh calls is caught.
    let convo = ctx.conversation_snapshot.read().await;
    assert_eq!(
        convo.messages.len(),
        expected_count,
        "busy conversation snapshot must advance with the conversation, not stay empty"
    );
    drop(convo);
    let stats = ctx.session_stats_snapshot.read().await;
    assert_eq!(
        stats.total_messages, expected_count,
        "busy session_stats snapshot must reflect the post-chain message count"
    );
}

#[test]
fn workflow_progress_fingerprint_changes_with_step_progress() {
    let mut env = DispatchTestEnv::with_selected_feature();
    let workflow = env.workflow.clone();
    let ctx = env.ctx();

    let before = super::workflow_progress_fingerprint(&ctx).unwrap();
    workflow.lock().unwrap().check(1).unwrap();
    let after = super::workflow_progress_fingerprint(&ctx).unwrap();
    assert_ne!(before, after);
}
