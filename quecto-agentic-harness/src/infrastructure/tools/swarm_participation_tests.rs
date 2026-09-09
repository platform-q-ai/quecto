//! #1715: swarm participation handle, engaged-workflow probe and the
//! membership-free run status (split from `swarm_bridge_tests.rs`).
use super::*;
use crate::domain::swarm::CoordinationPort;
use serde_json::json;

fn context(directory: &tempfile::TempDir) -> SwarmContext {
    SwarmContext {
        checkout: directory.path().to_path_buf(),
        member: "coordinator".into(),
        lifecycle: std::sync::Arc::new(crate::application::swarm::LifecycleService),
    }
}

/// #1715: a run cannot be created while the composition's workflow is
/// engaged; an idle available engine, or no engine at all, is not engaged.
#[test]
fn workflow_engaged_reflects_guards_and_selected_templates_only() {
    use crate::domain::workflow::{WorkflowConfig, WorkflowEngine};
    let slot: WorkflowEngineSlot = Default::default();
    assert!(!workflow_engaged(&slot), "no engine");
    let template = crate::domain::workflow::WorkflowTemplate {
        id: "t".into(),
        label: "t".into(),
        description: "test".into(),
        when_to_use: None,
        steps: vec![crate::domain::workflow::WorkflowTemplateStep {
            key: "a".into(),
            label: "A".into(),
            phase: "x".into(),
            guidance: None,
        }],
        guards: vec![],
    };
    let idle = std::sync::Arc::new(std::sync::Mutex::new(
        WorkflowEngine::new(
            WorkflowConfig {
                templates: vec![template.clone()],
                ..WorkflowConfig::default()
            },
            false,
        )
        .unwrap(),
    ));
    let _ = slot.set(idle.clone());
    assert!(!workflow_engaged(&slot), "idle engine");
    idle.lock().unwrap().select_template("t", None).unwrap();
    assert!(workflow_engaged(&slot), "selected template");
    let guarded: WorkflowEngineSlot = Default::default();
    let _ = guarded.set(std::sync::Arc::new(std::sync::Mutex::new(
        WorkflowEngine::new(
            WorkflowConfig {
                templates: vec![template],
                ..WorkflowConfig::default()
            },
            true,
        )
        .unwrap(),
    )));
    assert!(workflow_engaged(&guarded), "guards on");
}

/// #1715: a shared handle runs its hooks exactly once on the transition into
/// participation (immediately when already participating); a fixed answer
/// never transitions and never revokes.
#[test]
fn participation_hooks_run_once_on_the_transition_and_never_revoke() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let shared = Participation::shared();
    let runs = std::sync::Arc::new(AtomicUsize::new(0));
    let counter = runs.clone();
    shared.on_participation(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    shared.set(false);
    assert_eq!(runs.load(Ordering::SeqCst), 0);
    shared.set(true);
    shared.set(true);
    assert_eq!(runs.load(Ordering::SeqCst), 1, "once");
    shared.set(false);
    assert!(shared.participating(), "never revoked");
    let counter = runs.clone();
    shared.on_participation(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    assert_eq!(runs.load(Ordering::SeqCst), 2, "late hook runs at once");
    let fixed = Participation::Fixed(false);
    let counter = runs.clone();
    fixed.on_participation(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    fixed.set(true);
    assert!(!fixed.participating());
    assert_eq!(runs.load(Ordering::SeqCst), 2);
    assert_eq!(format!("{shared:?}"), "Participation(true)");
}

/// #1715: the membership-free status read tells a created run from the
/// bootstrap placeholder without touching the member table.
#[test]
fn run_created_reads_the_store_without_membership() {
    let directory = tempfile::tempdir().unwrap();
    let parent = context(&directory);
    assert!(!parent.run_created().unwrap(), "no store yet");
    std::fs::create_dir_all(directory.path().join(".quecto")).unwrap();
    parent
        .call("_bootstrap", json!([123, "start", "/parent", null]))
        .unwrap();
    let stranger = SwarmContext {
        member: "stranger".into(),
        ..parent.clone()
    };
    assert!(!stranger.run_created().unwrap(), "placeholder run");
    let deadline = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 300;
    parent
        .call(
            "create",
            json!(["ship", [], [{"id":"t","kind":"command","description":"pass"}], 1, deadline]),
        )
        .unwrap();
    assert!(
        stranger.run_created().unwrap(),
        "created run, still no membership"
    );
    assert!(
        parent
            .snapshot()
            .unwrap()
            .members
            .iter()
            .all(|m| m.id != "stranger"),
        "the status read added no member"
    );
}
