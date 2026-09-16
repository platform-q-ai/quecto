//! Contract for the `WorkflowRunSource` port (#1860, D5 #1972): the run to
//! record with the session, `None` while the engine holds nothing worth
//! restoring, and the engine's own persisted run once it does.
use std::sync::{Arc, Mutex};

use quecto::application::sessions::ports::WorkflowRunSource;
use quecto::domain::workflow::{
    WorkflowConfig, WorkflowEngine, WorkflowTemplate, WorkflowTemplateStep,
};
use quecto::infrastructure::persistence::session_snapshot_sources::WorkflowEngineRunSource;

fn engine() -> Arc<Mutex<WorkflowEngine>> {
    let config = WorkflowConfig {
        templates: vec![WorkflowTemplate {
            id: "feature".into(),
            label: "Feature".into(),
            description: "contract".into(),
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
    Arc::new(Mutex::new(WorkflowEngine::new(config, false).unwrap()))
}

fn under_test(engine: Arc<Mutex<WorkflowEngine>>) -> Arc<dyn WorkflowRunSource> {
    Arc::new(WorkflowEngineRunSource::new(engine))
}

#[test]
fn an_idle_engine_has_no_run_to_record() {
    assert_eq!(under_test(engine()).persisted_run(), None);
}

#[test]
fn a_selected_template_is_reported_as_the_engine_persists_it() {
    let engine = engine();
    let port = under_test(engine.clone());
    engine
        .lock()
        .unwrap()
        .select_template("feature", Some((7, "bug".into())))
        .unwrap();
    engine.lock().unwrap().check(1).unwrap();
    let expected = engine.lock().unwrap().persisted_run();
    assert!(expected.is_some());
    assert_eq!(port.persisted_run(), expected);
}

#[test]
fn a_reset_engine_reports_nothing_again() {
    let engine = engine();
    let port = under_test(engine.clone());
    engine
        .lock()
        .unwrap()
        .select_template("feature", None)
        .unwrap();
    assert!(port.persisted_run().is_some());
    engine.lock().unwrap().reset();
    assert_eq!(port.persisted_run(), None);
}
