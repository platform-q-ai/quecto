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
    let ids: Vec<&str> = snap
        .available_templates
        .iter()
        .map(|t| t.id.as_str())
        .collect();
    assert_eq!(ids, ["feature", "refactor"]);
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
    assert_eq!(snap.progress.total, 19);
    assert_eq!(snap.current_step.unwrap().key, "hooks");
    assert_eq!(snap.steps.len(), 19);
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
    assert!(nudge.contains("Never ask for permission"));
    // Literal instruction-followers (e.g. GPT-5.6) treated the old "Respond
    // with just the word DONE" sentence as a status poll with a mandated
    // one-word answer — a no-tool-call reply the no-progress detector then
    // read as a stall, silently killing auto-continue mid-run.
    assert!(
        !nudge.contains("DONE"),
        "nudge must not mandate a one-word DONE reply: {nudge}"
    );
    assert!(
        !nudge.contains("Respond with just the word"),
        "nudge must not mandate a one-word status reply: {nudge}"
    );
    // Error path: after a failed tool call the model needs an instruction
    // other than "never stop" — retry/work around, or name the blocked step.
    assert!(
        nudge.contains("If a tool call failed, retry or work around it"),
        "nudge must carry an error-path instruction: {nudge}"
    );
    assert!(
        nudge.contains("state which step is blocked and why"),
        "nudge must tell a blocked model to name the blocked step: {nudge}"
    );
}

#[test]
fn corrective_nudge_demands_check_off_or_continued_work() {
    let config = WorkflowConfig {
        auto_continue: true,
        ..WorkflowConfig::default()
    };
    let mut engine = WorkflowEngine::new(config, false).unwrap();
    engine.select_template("feature", None).unwrap();
    let nudge = engine.corrective_nudge().unwrap();
    assert!(
        nudge.contains("did not advance the workflow"),
        "corrective nudge must name the stall: {nudge}"
    );
    // Pins the tool reference so a rename/reword of the check-off
    // instruction in the sibling nudges cannot leave this one stale.
    assert!(
        nudge.contains("check it off with the workflow tool"),
        "corrective nudge must point at the workflow tool for the check-off: {nudge}"
    );
    assert!(
        nudge.contains("Do not reply with only a status message"),
        "corrective nudge must forbid a bare status reply: {nudge}"
    );
}

#[test]
fn corrective_nudge_shares_the_auto_continue_gate() {
    // Disabled auto-continue: no corrective nudge either.
    let disabled = WorkflowConfig {
        auto_continue: false,
        ..WorkflowConfig::default()
    };
    let mut engine = WorkflowEngine::new(disabled, false).unwrap();
    engine.select_template("feature", None).unwrap();
    assert!(engine.corrective_nudge().is_none());

    // Enabled but complete: gate closes exactly like the standard nudge's.
    let config = WorkflowConfig {
        auto_continue: true,
        ..WorkflowConfig::default()
    };
    let mut engine = WorkflowEngine::new(config, false).unwrap();
    engine.select_template("feature", None).unwrap();
    assert!(engine.corrective_nudge().is_some());
    let total = engine.progress().total;
    for step in 1..=total {
        engine.check(step).unwrap();
    }
    assert!(engine.auto_continue_nudge().is_none());
    assert!(engine.corrective_nudge().is_none());
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
    assert!(status.contains("INLINE review comment"));
    assert!(status.contains("addPullRequestReview"));
}
