use super::*;

#[test]
fn restore_with_missing_template_resets_to_selector() {
    let mut engine = WorkflowEngine::new(WorkflowConfig::default(), false).unwrap();
    engine.restore_run(WorkflowRunPersisted {
        template_id: Some("missing".into()),
        done: vec![true],
        active_issue: Some((1, "x".into())),
    });
    assert_eq!(engine.mode(), WorkflowMode::SelectingTemplate);
    assert!(engine.current_step().is_none());
}

#[test]
fn snapshot_in_selector_mode_lists_templates() {
    let engine = WorkflowEngine::new(WorkflowConfig::default(), false).unwrap();
    let snap = engine.snapshot(true);
    assert_eq!(snap.mode, WorkflowMode::SelectingTemplate);
    assert!(snap.available_templates.len() >= 4);
    assert!(snap.steps.is_empty());
}

#[test]
fn snapshot_in_active_mode_has_steps_and_current_step() {
    let mut engine = WorkflowEngine::new(WorkflowConfig::default(), true).unwrap();
    engine
        .select_template("feature", Some((9, "feat".into())))
        .unwrap();
    let snap = engine.snapshot(true);
    assert_eq!(snap.mode, WorkflowMode::Active);
    assert_eq!(snap.progress.total, 7);
    assert_eq!(snap.current_step.unwrap().key, "scenarios");
    assert_eq!(snap.steps.len(), 7);
    assert!(snap.guards_enabled);
}

#[test]
fn selector_prompt_mentions_select_template() {
    let engine = WorkflowEngine::new(WorkflowConfig::default(), false).unwrap();
    let prompt = engine.prompt_snippet();
    assert!(prompt.contains("select_template"));
    assert!(prompt.contains("feature"));
}

#[test]
fn active_prompt_mentions_guidance() {
    let mut engine = WorkflowEngine::new(WorkflowConfig::default(), false).unwrap();
    engine.select_template("fix", None).unwrap();
    let prompt = engine.prompt_snippet();
    assert!(prompt.contains("CURRENT STEP"));
    assert!(prompt.contains("reproducing the bug"));
}

#[test]
fn auto_continue_nudge_uses_continuation_wording() {
    let mut engine = WorkflowEngine::new(WorkflowConfig::default(), false).unwrap();
    engine.select_template("feature", None).unwrap();
    let nudge = engine.auto_continue_nudge().unwrap();
    assert!(nudge.contains("Workflow incomplete."));
    assert!(nudge.contains("Continue with the next incomplete step."));
    assert!(nudge.contains("Respond with just the word DONE"));
    assert!(nudge.contains("Never ask for permission"));
}

#[test]
fn no_active_template_errors_for_step_actions() {
    let mut engine = WorkflowEngine::new(WorkflowConfig::default(), false).unwrap();
    let err = engine.check(1).unwrap_err();
    assert!(matches!(err, WorkflowError::NoActiveTemplate(_)));
}
