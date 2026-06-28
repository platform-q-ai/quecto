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
    assert_eq!(snap.available_templates.len(), 1);
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
    assert_eq!(snap.progress.total, 17);
    assert_eq!(snap.current_step.unwrap().key, "hooks");
    assert_eq!(snap.steps.len(), 17);
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
    engine.select_template("feature", None).unwrap();
    engine.check(1).unwrap();
    let prompt = engine.prompt_snippet();
    assert!(prompt.contains("CURRENT STEP"));
    assert!(prompt.contains("acceptance criteria"));
}

#[test]
fn auto_continue_nudge_uses_continuation_wording() {
    let config = WorkflowConfig {
        auto_continue: true,
        ..WorkflowConfig::default()
    };
    let mut engine = WorkflowEngine::new(config, false).unwrap();
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

#[test]
fn status_text_shows_guidance_for_incomplete_non_current_steps() {
    // Regression: the status view used to render guidance only for the CURRENT
    // step, so an agent reading the workflow ahead of time saw later steps as
    // bare labels (e.g. the reviewers step). Now every INCOMPLETE step shows its
    // guidance, while completed steps stay compact.
    let mut engine = WorkflowEngine::new(WorkflowConfig::default(), false).unwrap();
    engine.select_template("feature", None).unwrap();
    // Current step is step 1 (hooks); reviewers is a later, non-current step.
    let status = engine.status_text();
    assert!(status.contains("CURRENT STEP"));
    // An upcoming, non-current step's guidance is visible:
    assert!(status.contains("INLINE review comments"));
    assert!(status.contains("addPullRequestReview"));
}
