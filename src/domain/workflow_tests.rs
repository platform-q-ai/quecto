use super::*;

#[test]
fn default_config_uses_builtins_when_templates_empty() {
    let engine = WorkflowEngine::new(WorkflowConfig::default(), false).unwrap();
    let templates = engine.list_templates();
    assert!(templates.iter().any(|t| t.id == "feature"));
    assert!(templates.iter().any(|t| t.id == "fix"));
    assert!(templates.iter().any(|t| t.id == "refactor"));
    assert!(templates.iter().any(|t| t.id == "chore"));
}

#[test]
fn selector_mode_before_template_selection() {
    let engine = WorkflowEngine::new(WorkflowConfig::default(), false).unwrap();
    assert_eq!(engine.mode(), WorkflowMode::SelectingTemplate);
    assert!(engine.status_text().contains("Available templates"));
}

#[test]
fn select_template_starts_run() {
    let mut engine = WorkflowEngine::new(WorkflowConfig::default(), false).unwrap();
    engine.select_template("fix", None).unwrap();
    assert_eq!(engine.mode(), WorkflowMode::Active);
    assert_eq!(engine.progress().total, 6);
    assert_eq!(engine.current_step().unwrap().index, 1);
}

#[test]
fn check_enforces_ordering() {
    let mut engine = WorkflowEngine::new(WorkflowConfig::default(), false).unwrap();
    engine.select_template("feature", None).unwrap();
    let err = engine.check(3).unwrap_err();
    assert!(err.to_string().contains("complete step 1"));
}

#[test]
fn check_and_uncheck_work() {
    let mut engine = WorkflowEngine::new(WorkflowConfig::default(), false).unwrap();
    engine.select_template("feature", None).unwrap();
    engine.check(1).unwrap();
    engine.check(2).unwrap();
    assert_eq!(engine.progress().done, 2);
    engine.uncheck(2).unwrap();
    assert_eq!(engine.progress().done, 1);
}

#[test]
fn skip_bypasses_ordering() {
    let mut engine = WorkflowEngine::new(WorkflowConfig::default(), false).unwrap();
    engine.select_template("feature", None).unwrap();
    engine.skip(5).unwrap();
    assert_eq!(engine.progress().done, 1);
}

#[test]
fn reset_returns_to_selector_mode() {
    let mut engine = WorkflowEngine::new(WorkflowConfig::default(), false).unwrap();
    engine
        .select_template("feature", Some((42, "test".into())))
        .unwrap();
    engine.check(1).unwrap();
    engine.reset();
    assert_eq!(engine.mode(), WorkflowMode::SelectingTemplate);
    assert!(engine.persisted_run().is_none());
}

#[test]
fn set_issue_truncates_title() {
    let mut engine = WorkflowEngine::new(WorkflowConfig::default(), false).unwrap();
    let long = "x".repeat(600);
    engine.set_issue(1, long);
    assert!(engine.snapshot(true).active_issue.unwrap().1.len() <= 500);
}

#[test]
fn persisted_run_round_trip() {
    let mut engine = WorkflowEngine::new(WorkflowConfig::default(), false).unwrap();
    engine
        .select_template("fix", Some((7, "bug".into())))
        .unwrap();
    engine.check(1).unwrap();
    let persisted = engine.persisted_run().unwrap();

    let mut restored = WorkflowEngine::new(WorkflowConfig::default(), false).unwrap();
    restored.restore_run(persisted);
    assert_eq!(restored.mode(), WorkflowMode::Active);
    assert_eq!(restored.progress().done, 1);
    assert_eq!(
        restored.snapshot(true).active_issue,
        Some((7, "bug".into()))
    );
}

#[test]
fn persisted_run_exists_for_issue_without_selected_template() {
    let mut engine = WorkflowEngine::new(WorkflowConfig::default(), false).unwrap();
    engine.set_issue(99, "triage".into());
    let persisted = engine
        .persisted_run()
        .expect("issue-only state should persist");
    assert_eq!(persisted.template_id, None);
    assert_eq!(persisted.active_issue, Some((99, "triage".into())));

    let mut restored = WorkflowEngine::new(WorkflowConfig::default(), false).unwrap();
    restored.restore_run(persisted);
    assert_eq!(
        restored.snapshot(true).active_issue,
        Some((99, "triage".into()))
    );
    assert_eq!(restored.mode(), WorkflowMode::SelectingTemplate);
}

#[test]
fn restore_run_clears_ordering_gaps() {
    let mut engine = WorkflowEngine::new(WorkflowConfig::default(), false).unwrap();
    engine.restore_run(WorkflowRunPersisted {
        template_id: Some("feature".into()),
        done: vec![true, false, true, true],
        active_issue: Some((1, "gap".into())),
    });
    let snapshot = engine.snapshot(true);
    assert_eq!(snapshot.progress.done, 1);
    assert_eq!(snapshot.current_step.unwrap().index, 2);
    assert_eq!(snapshot.steps[2].done, false);
    assert_eq!(snapshot.steps[3].done, false);
}

#[test]
fn guards_block_until_before_step_key_threshold() {
    let mut engine = WorkflowEngine::new(WorkflowConfig::default(), true).unwrap();
    engine.select_template("feature", None).unwrap();
    let err = engine.check_guards().unwrap_err();
    assert!(err.to_string().contains("Complete step 1"));
    for step in 1..=6 {
        engine.check(step).unwrap();
    }
    assert!(engine.check_guards().is_ok());
}

#[test]
fn completion_nudge_only_when_complete() {
    let mut engine = WorkflowEngine::new(WorkflowConfig::default(), false).unwrap();
    engine.select_template("chore", None).unwrap();
    assert!(engine.completion_nudge().is_none());
    for step in 1..=4 {
        engine.check(step).unwrap();
    }
    assert!(
        engine
            .completion_nudge()
            .unwrap()
            .contains("All workflow steps complete")
    );
}

#[test]
fn validate_duplicate_template_ids() {
    let cfg = WorkflowConfig {
        templates: vec![
            WorkflowTemplate {
                id: "x".into(),
                label: "X".into(),
                description: "x".into(),
                when_to_use: None,
                steps: vec![WorkflowTemplateStep {
                    key: "a".into(),
                    label: "A".into(),
                    phase: "red".into(),
                    guidance: None,
                }],
                guards: vec![],
            },
            WorkflowTemplate {
                id: "x".into(),
                label: "Y".into(),
                description: "y".into(),
                when_to_use: None,
                steps: vec![WorkflowTemplateStep {
                    key: "b".into(),
                    label: "B".into(),
                    phase: "green".into(),
                    guidance: None,
                }],
                guards: vec![],
            },
        ],
        ..Default::default()
    };
    assert!(WorkflowEngine::new(cfg, false).is_err());
}

#[test]
fn validate_guard_unknown_step_key() {
    let cfg = WorkflowConfig {
        templates: vec![WorkflowTemplate {
            id: "x".into(),
            label: "X".into(),
            description: "x".into(),
            when_to_use: None,
            steps: vec![WorkflowTemplateStep {
                key: "a".into(),
                label: "A".into(),
                phase: "red".into(),
                guidance: None,
            }],
            guards: vec![WorkflowGuardRule {
                commands: vec!["git commit".into()],
                before_step_key: "missing".into(),
                message: "blocked".into(),
            }],
        }],
        ..Default::default()
    };
    assert!(WorkflowEngine::new(cfg, false).is_err());
}
