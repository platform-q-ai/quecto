use super::{RegistryRosterSource, WorkflowEngineRunSource};
use crate::application::sessions::ports::{HistoricalRosterSource, WorkflowRunSource};
use crate::domain::ids::AgentUuid;
use crate::domain::session::{SubagentLiveness, SubagentRestoreReason};
use crate::domain::workflow::{
    WorkflowConfig, WorkflowEngine, WorkflowTemplate, WorkflowTemplateStep,
};
use crate::infrastructure::tools::subagent_registry::{SubagentEntry, new_registry};

fn engine() -> std::sync::Arc<std::sync::Mutex<WorkflowEngine>> {
    let config = WorkflowConfig {
        templates: vec![WorkflowTemplate {
            id: "feature".into(),
            label: "Feature".into(),
            description: "test".into(),
            when_to_use: None,
            steps: vec![WorkflowTemplateStep {
                key: "s1".into(),
                label: "Step 1".into(),
                phase: "test".into(),
                guidance: None,
            }],
            guards: vec![],
        }],
        ..WorkflowConfig::default()
    };
    std::sync::Arc::new(std::sync::Mutex::new(
        WorkflowEngine::new(config, false).unwrap(),
    ))
}

#[test]
fn the_workflow_source_reports_the_engine_run_or_nothing() {
    let engine = engine();
    let source = WorkflowEngineRunSource::new(engine.clone());
    assert_eq!(source.persisted_run(), None);
    engine
        .lock()
        .unwrap()
        .select_template("feature", Some((3, "issue".into())))
        .unwrap();
    let run = source
        .persisted_run()
        .expect("a selected template persists");
    assert_eq!(run.template_id.as_deref(), Some("feature"));
    assert_eq!(run.active_issue, Some((3, "issue".into())));
}

#[test]
fn the_workflow_source_reports_nothing_over_a_poisoned_engine_lock() {
    let engine = engine();
    let shared = engine.clone();
    let _ = std::thread::spawn(move || {
        let _guard = shared.lock().unwrap();
        panic!("poison for coverage");
    })
    .join();
    assert_eq!(WorkflowEngineRunSource::new(engine).persisted_run(), None);
}

#[test]
fn the_roster_source_maps_rows_as_history_without_pid_or_socket() {
    let registry = new_registry();
    {
        let mut entries = registry.lock().unwrap();
        let mut b = SubagentEntry::with_identity(
            AgentUuid::from("b".to_string()),
            "beta".to_string(),
            "/tmp/b.sock".into(),
            20,
        );
        b.persisted_liveness = SubagentLiveness::Dead;
        b.read_only = true;
        b.parent_id = Some("root".to_string());
        entries.insert("b".to_string(), b);
        entries.insert(
            "legacy-key".to_string(),
            SubagentEntry::new("/tmp/legacy.sock".into(), 4242),
        );
    }
    let mut rows = RegistryRosterSource::new(registry).roster_rows();
    rows.sort_by(|x, y| x.display_name.cmp(&y.display_name));
    assert_eq!(rows.len(), 2);
    let beta = rows.iter().find(|r| r.agent_uuid == "b").unwrap();
    assert_eq!(beta.display_name, "beta");
    assert_eq!(beta.session_key, "b");
    assert_eq!(beta.liveness, SubagentLiveness::Dead);
    assert_eq!(
        beta.restore_reason,
        SubagentRestoreReason::LegacyUnspecified
    );
    assert_eq!(beta.parent_id.as_deref(), Some("root"));
    assert!(beta.read_only);
    assert!(beta.status.is_some());
    let legacy = rows.iter().find(|r| r.agent_uuid != "b").unwrap();
    assert_eq!(
        legacy.display_name, "legacy-key",
        "a registry keyed by display name keeps that label"
    );
    let json = serde_json::to_string(&rows).unwrap();
    assert!(!json.contains("pid"), "{json}");
    assert!(!json.contains("socketPath"), "{json}");
    assert!(!json.contains("/tmp/"), "{json}");
    assert!(!json.contains("4242"), "{json}");
}

#[test]
fn the_roster_source_recovers_from_a_poisoned_registry_lock() {
    let registry = new_registry();
    registry.lock().unwrap().insert(
        "worker".into(),
        SubagentEntry::new("/tmp/worker.sock".into(), 0),
    );
    let shared = registry.clone();
    let _ = std::thread::spawn(move || {
        let _guard = shared.lock().unwrap();
        panic!("poison registry for coverage");
    })
    .join();
    assert!(registry.lock().is_err());
    assert_eq!(RegistryRosterSource::new(registry).roster_rows().len(), 1);
}
