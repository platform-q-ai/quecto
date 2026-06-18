use super::*;

#[test]
fn default_config_uses_builtins_when_templates_empty() {
    let engine = WorkflowEngine::new(WorkflowConfig::default(), false).unwrap();
    let templates = engine.list_templates();
    assert_eq!(templates.len(), 1);
    assert_eq!(templates[0].id, "feature");
}

#[test]
fn selector_mode_before_template_selection() {
    let engine = WorkflowEngine::new(WorkflowConfig::default(), false).unwrap();
    assert_eq!(engine.mode(), WorkflowMode::SelectingTemplate);
    assert!(engine.status_text().contains("Available templates"));
}

#[test]
fn default_workflow_config_enables_core_backend_nudges() {
    let engine = WorkflowEngine::new(WorkflowConfig::default(), false).unwrap();
    assert!(engine.auto_continue_enabled());
    assert!(engine.completion_nudge_enabled());
}

#[test]
fn select_template_starts_run() {
    let mut engine = WorkflowEngine::new(WorkflowConfig::default(), false).unwrap();
    engine.select_template("feature", None).unwrap();
    assert_eq!(engine.mode(), WorkflowMode::Active);
    assert_eq!(engine.progress().total, 17);
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
        .select_template("feature", Some((7, "bug".into())))
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
fn workflow_subsystem_select_template_preserves_issue_set_in_selector_mode() {
    let mut engine = WorkflowEngine::new(WorkflowConfig::default(), false).unwrap();
    engine.set_issue(42, "keep me".into());

    engine.select_template("feature", None).unwrap();

    assert_eq!(
        engine.snapshot(true).active_issue,
        Some((42, "keep me".into()))
    );
}

#[test]
fn workflow_subsystem_select_template_explicit_issue_overrides_existing_one() {
    let mut engine = WorkflowEngine::new(WorkflowConfig::default(), false).unwrap();
    engine.set_issue(1, "old issue".into());

    engine
        .select_template("feature", Some((2, "new issue".into())))
        .unwrap();

    assert_eq!(
        engine.snapshot(true).active_issue,
        Some((2, "new issue".into()))
    );
}

#[test]
fn restore_run_unknown_template_recovers_to_selector_mode() {
    let mut engine = WorkflowEngine::new(WorkflowConfig::default(), false).unwrap();
    engine.restore_run(WorkflowRunPersisted {
        template_id: Some("deleted_template".into()),
        done: vec![true, false],
        active_issue: None,
    });
    assert_eq!(engine.mode(), WorkflowMode::SelectingTemplate);
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
    assert!(!snapshot.steps[2].done);
    assert!(!snapshot.steps[3].done);
}

#[test]
fn guards_block_until_before_step_key_threshold() {
    let mut engine = WorkflowEngine::new(WorkflowConfig::default(), true).unwrap();
    engine.select_template("feature", None).unwrap();
    let err = engine.check_guards().unwrap_err();
    assert!(err.to_string().contains("Complete step 1"));
    for step in 1..=17 {
        engine.check(step).unwrap();
    }
    assert!(engine.check_guards().is_ok());
}

#[test]
fn completion_nudge_only_when_complete() {
    let config = WorkflowConfig {
        completion_nudge: true,
        ..WorkflowConfig::default()
    };
    let mut engine = WorkflowEngine::new(config, false).unwrap();
    engine.select_template("feature", None).unwrap();
    assert!(engine.completion_nudge().is_none());
    for step in 1..=17 {
        engine.check(step).unwrap();
    }
    let nudge = engine.completion_nudge().unwrap();
    assert!(nudge.contains("All workflow steps complete"));
    assert!(nudge.contains("issues authored by the authenticated user only"));
    assert!(nudge.contains("gh issue list --author @me"));
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

#[test]
fn default_feature_template_matches_config_file_quecto_feature_workflow_with_hook_step() {
    let mut engine = WorkflowEngine::new(WorkflowConfig::default(), false).unwrap();
    engine.select_template("feature", None).unwrap();
    let snap = engine.snapshot(true);

    let keys: Vec<&str> = snap.steps.iter().map(|s| s.key.as_str()).collect();
    assert_eq!(
        keys,
        vec![
            "hooks",
            "scenarios",
            "tests",
            "red",
            "green",
            "refactor",
            "verify",
            "commit",
            "push",
            "pr",
            "reviewers",
            "fix_reviews",
            "push_fixes",
            "resolve_threads",
            "pre_merge",
            "merge",
            "pull",
        ]
    );
    assert_eq!(snap.progress.total, 17);
    assert_eq!(snap.steps[0].label, "Install/check local quality hooks");
    assert_eq!(snap.steps[1].label, "Update Scenarios / Add new features");
    assert_eq!(snap.steps[5].label, "Refactor");
    assert_eq!(snap.steps[6].label, "Ensure tests still pass");
    assert_eq!(
        snap.steps[8].label,
        "Push (pre-push hook will run tests and linting)"
    );
    assert_eq!(
        snap.steps[10].label,
        "Despatch sub agents in parallel as reviewers (Architecture, Security and Performance)"
    );
    assert_eq!(snap.steps[15].label, "Merge");
    assert_eq!(snap.steps[16].label, "Move to local master and pull");
}

#[test]
fn feature_template_guards_commit_push_and_merge_like_config_file() {
    let templates = default_templates();
    let feature = templates
        .iter()
        .find(|template| template.id == "feature")
        .expect("feature template exists");

    assert_eq!(feature.guards.len(), 2);
    assert_eq!(
        feature.guards[0].commands,
        vec!["git commit".to_string(), "git push".to_string()]
    );
    assert_eq!(feature.guards[0].before_step_key, "commit");
    assert!(
        feature.guards[0]
            .message
            .contains("Complete hook setup and RED/GREEN work")
    );
    assert_eq!(
        feature.guards[1].commands,
        vec!["git merge".to_string(), "gh pr merge".to_string()]
    );
    assert_eq!(feature.guards[1].before_step_key, "merge");
    assert!(feature.guards[1].message.contains("Complete code review"));
}
