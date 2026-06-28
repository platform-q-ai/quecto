use super::*;

#[test]
fn workflow_spec_deserializes_with_template_and_ignores_extra_fields() {
    let json = serde_json::json!({
        "template": {
            "id": "review",
            "label": "Review",
            "description": "desc",
            "steps": [{"key": "a", "label": "A", "phase": "review"}]
        },
        "inputs": {"pr": 7},        // forward-compat extras must be ignored
        "acceptance": "tests pass"
    });
    let spec: WorkflowSpec = serde_json::from_value(json).unwrap();
    assert_eq!(spec.template.id, "review");
    assert_eq!(spec.template.steps.len(), 1);
}

#[test]
fn workflow_spec_requires_a_template() {
    let json = serde_json::json!({ "inputs": {"pr": 7} });
    assert!(serde_json::from_value::<WorkflowSpec>(json).is_err());
}

#[test]
fn single_template_config_binds_to_active_on_select() {
    let template = WorkflowTemplate {
        id: "review".into(),
        label: "Review".into(),
        description: "desc".into(),
        when_to_use: None,
        steps: vec![WorkflowTemplateStep {
            key: "a".into(),
            label: "A".into(),
            phase: "review".into(),
            guidance: None,
        }],
        guards: vec![],
    };
    let config = WorkflowConfig {
        auto_continue: true,
        completion_nudge: true,
        selector_prompt: None,
        templates: vec![template],
    };
    let mut engine = WorkflowEngine::new(config, false).unwrap();
    // Before selection the engine is in selector mode...
    assert_eq!(engine.mode(), WorkflowMode::SelectingTemplate);
    // ...and binding it to the only template activates it immediately.
    engine.select_template("review", None).unwrap();
    assert_eq!(engine.mode(), WorkflowMode::Active);
    assert_eq!(engine.list_templates().len(), 1);
}

fn bound_template(id: &str) -> WorkflowTemplate {
    WorkflowTemplate {
        id: id.into(),
        label: id.into(),
        description: "d".into(),
        when_to_use: None,
        steps: vec![WorkflowTemplateStep {
            key: "a".into(),
            label: "A".into(),
            phase: "x".into(),
            guidance: None,
        }],
        guards: vec![],
    }
}

fn bound_engine(templates: Vec<WorkflowTemplate>, select: &str) -> WorkflowEngine {
    let config = WorkflowConfig {
        auto_continue: true,
        completion_nudge: true,
        selector_prompt: None,
        templates,
    };
    let mut engine = WorkflowEngine::new(config, false).unwrap();
    engine.select_template(select, None).unwrap();
    engine.set_bound(true);
    engine
}

#[test]
fn bound_engine_reset_keeps_template_active() {
    let mut engine = bound_engine(vec![bound_template("only")], "only");
    engine.check(1).unwrap();
    engine.reset();
    // A bound engine must NOT return to template selection on reset; it stays
    // active on the assigned template with progress cleared.
    assert_eq!(engine.mode(), WorkflowMode::Active);
    assert!(engine.is_bound());
}

#[test]
fn bound_engine_rejects_switching_template() {
    let mut engine = bound_engine(vec![bound_template("a"), bound_template("b")], "a");
    assert!(
        engine.select_template("b", None).is_err(),
        "a bound engine must not switch to a different template"
    );
    // Re-selecting the SAME bound template is allowed (reset relies on it).
    assert!(engine.select_template("a", None).is_ok());
}

#[test]
fn bound_engine_completion_nudge_does_not_instruct_reselect() {
    let mut engine = bound_engine(vec![bound_template("only")], "only");
    engine.check(1).unwrap();
    assert_eq!(engine.mode(), WorkflowMode::Complete);
    let nudge = engine
        .completion_nudge()
        .expect("a completed bound workflow should still nudge");
    assert!(
        !nudge.contains("select_template") && !nudge.contains("reset"),
        "bound completion nudge must not tell the model to reset/reselect: {nudge}"
    );
}

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
    let step_count = engine.progress().total;
    for step in 1..=step_count {
        engine.check(step).unwrap();
    }
    let nudge = engine.completion_nudge().unwrap();
    assert!(nudge.contains("All workflow steps complete"));
    assert!(nudge.contains("report your result and stop"));
    // #885: an unbound completion must NOT tell the agent to self-select the
    // next issue or restart the workflow — the master agent owns issue choice.
    assert!(!nudge.contains("issues authored by the authenticated user only"));
    assert!(!nudge.contains("gh issue list --author @me"));
    assert!(!nudge.contains("select_template"));
    assert!(!nudge.contains("reset"));
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
            "bdd_review",
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
            "cleanup",
        ]
    );
    assert_eq!(snap.progress.total, 17);
    assert_eq!(snap.steps[0].label, "Install/check local quality hooks");
    assert_eq!(snap.steps[1].label, "Update Scenarios / Add new features");
    assert_eq!(
        snap.steps[3].label,
        "Ensure new/modified tests FAIL (RED) — quick targeted run only, not full suite"
    );
    assert_eq!(
        snap.steps[4].label,
        "Despatch BDD sub-agent to review BDD feature, step tests and unit tests"
    );
    assert_eq!(snap.steps[6].label, "Refactor");
    assert_eq!(snap.steps[7].label, "Ensure tests still pass");
    assert_eq!(
        snap.steps[9].label,
        "Push (pre-push hook will run tests and linting)"
    );
    assert_eq!(
        snap.steps[11].label,
        "Despatch sub agents in parallel as reviewers (Architecture, Security and Performance)"
    );
    assert_eq!(
        snap.steps[15].label,
        "Confirm the pre-push gate passed and report the PR (do NOT merge)"
    );
    assert_eq!(snap.steps[16].label, "Clean up sub agents");
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
    assert_eq!(feature.guards[1].before_step_key, "cleanup");
    assert!(feature.guards[1].message.contains("Complete code review"));
}

#[test]
fn workflow_mode_wire_str_matches_serde_snake_case() {
    for mode in [
        WorkflowMode::SelectingTemplate,
        WorkflowMode::Active,
        WorkflowMode::Complete,
    ] {
        let json = serde_json::to_value(mode).unwrap();
        assert_eq!(json, serde_json::Value::String(mode.wire_str().to_string()));
    }
}

#[test]
fn verdict_status_round_trips_through_serde() {
    for (status, wire) in [
        (VerdictStatus::Completed, "completed"),
        (VerdictStatus::Failed, "failed"),
        (VerdictStatus::Incomplete, "incomplete"),
    ] {
        let json = serde_json::to_value(status).unwrap();
        assert_eq!(json, serde_json::json!(wire));
        let back: VerdictStatus = serde_json::from_value(json).unwrap();
        assert_eq!(back, status);
    }
}

#[test]
fn workflow_error_display_renders_inner_message() {
    let cases = [
        WorkflowError::UnknownTemplate("u".into()),
        WorkflowError::InvalidStep("i".into()),
        WorkflowError::OrderingViolation("o".into()),
        WorkflowError::NoActiveTemplate("n".into()),
        WorkflowError::InvalidConfig("c".into()),
        WorkflowError::GuardBlocked("g".into()),
    ];
    let rendered: Vec<String> = cases.iter().map(|e| e.to_string()).collect();
    assert_eq!(rendered, vec!["u", "i", "o", "n", "c", "g"]);
    // Exercise the std::error::Error impl too.
    let err: &dyn std::error::Error = &cases[0];
    assert_eq!(err.to_string(), "u");
}

#[test]
fn workflow_snapshot_round_trips_through_serde() {
    let snapshot = WorkflowSnapshot {
        enabled: true,
        guards_enabled: false,
        mode: WorkflowMode::Active,
        active_template: Some(WorkflowTemplateSummary {
            id: "feature".into(),
            label: "Feature".into(),
            description: "desc".into(),
            when_to_use: Some("when coding".into()),
        }),
        active_issue: Some((7, "title".into())),
        progress: WorkflowProgress {
            done: 1,
            total: 4,
            percent: 25,
        },
        current_step: Some(WorkflowStepStatus {
            index: 0,
            key: "red".into(),
            label: "RED".into(),
            phase: "red".into(),
            done: false,
            guidance: Some("write a failing test".into()),
        }),
        steps: vec![WorkflowStepStatus {
            index: 0,
            key: "red".into(),
            label: "RED".into(),
            phase: "red".into(),
            done: false,
            guidance: None,
        }],
        available_templates: vec![WorkflowTemplateSummary {
            id: "feature".into(),
            label: "Feature".into(),
            description: "desc".into(),
            when_to_use: None,
        }],
    };

    let json = serde_json::to_value(&snapshot).unwrap();
    let back: WorkflowSnapshot = serde_json::from_value(json).unwrap();
    assert_eq!(back, snapshot);
}

#[test]
fn workflow_run_default_is_empty() {
    let run = WorkflowRun::default();
    assert_eq!(run.template_id, None);
    assert_eq!(run.template_index, None);
    assert!(run.done.is_empty());
    assert_eq!(run.active_issue, None);
}

// ── WorkflowEngine validation error paths (config-level) ──────────────────

fn cfg(templates: Vec<WorkflowTemplate>) -> WorkflowConfig {
    WorkflowConfig {
        auto_continue: true,
        completion_nudge: true,
        selector_prompt: None,
        templates,
    }
}

fn step(key: &str) -> WorkflowTemplateStep {
    WorkflowTemplateStep {
        key: key.into(),
        label: key.to_uppercase(),
        phase: "x".into(),
        guidance: None,
    }
}

fn template_with_steps(id: &str, steps: Vec<WorkflowTemplateStep>) -> WorkflowTemplate {
    WorkflowTemplate {
        id: id.into(),
        label: id.into(),
        description: "d".into(),
        when_to_use: None,
        steps,
        guards: vec![],
    }
}

#[test]
fn new_rejects_too_many_templates() {
    let templates: Vec<WorkflowTemplate> = (0..33)
        .map(|i| template_with_steps(&format!("t{i}"), vec![step("a")]))
        .collect();
    let err = WorkflowEngine::new(cfg(templates), false).unwrap_err();
    assert!(matches!(err, WorkflowError::InvalidConfig(_)));
    assert!(err.to_string().contains("too many workflow templates"));
}

#[test]
fn new_rejects_empty_template_id() {
    let err = WorkflowEngine::new(cfg(vec![template_with_steps("", vec![step("a")])]), false)
        .unwrap_err();
    assert!(err.to_string().contains("template id cannot be empty"));
}

#[test]
fn new_rejects_template_with_no_steps() {
    let err = WorkflowEngine::new(cfg(vec![template_with_steps("t", vec![])]), false).unwrap_err();
    assert!(err.to_string().contains("has no steps"));
}

#[test]
fn new_rejects_template_with_too_many_steps() {
    let steps: Vec<WorkflowTemplateStep> = (0..101).map(|i| step(&format!("s{i}"))).collect();
    let err = WorkflowEngine::new(cfg(vec![template_with_steps("t", steps)]), false).unwrap_err();
    assert!(err.to_string().contains("too many steps"));
}

#[test]
fn new_rejects_step_with_empty_key() {
    let err = WorkflowEngine::new(cfg(vec![template_with_steps("t", vec![step("")])]), false)
        .unwrap_err();
    assert!(err.to_string().contains("empty key"));
}

#[test]
fn new_rejects_duplicate_step_key() {
    let err = WorkflowEngine::new(
        cfg(vec![template_with_steps("t", vec![step("a"), step("a")])]),
        false,
    )
    .unwrap_err();
    assert!(err.to_string().contains("duplicate step key"));
}

#[test]
fn new_rejects_guard_referencing_unknown_step_key() {
    let mut template = template_with_steps("t", vec![step("a")]);
    template.guards = vec![WorkflowGuardRule {
        commands: vec!["git push".into()],
        before_step_key: "nonexistent".into(),
        message: "blocked".into(),
    }];
    let err = WorkflowEngine::new(cfg(vec![template]), true).unwrap_err();
    assert!(
        err.to_string()
            .contains("guard references unknown step key")
    );
}

#[test]
fn guards_enabled_and_clear_issue_accessors() {
    let mut engine =
        WorkflowEngine::new(cfg(vec![template_with_steps("t", vec![step("a")])]), true).unwrap();
    assert!(engine.guards_enabled());
    engine.set_issue(7, "title".into());
    engine.clear_issue();
    let snap = engine.snapshot(true);
    assert_eq!(snap.active_issue, None);
}

#[test]
fn select_unknown_template_is_unknown_template_error() {
    let mut engine = WorkflowEngine::new(
        cfg(vec![template_with_steps("known", vec![step("a")])]),
        false,
    )
    .unwrap();
    let err = engine.select_template("does-not-exist", None).unwrap_err();
    assert!(matches!(err, WorkflowError::UnknownTemplate(_)));
    assert!(err.to_string().contains("unknown template"));
}

#[test]
fn check_invalid_step_index_is_invalid_step_error() {
    let mut engine =
        WorkflowEngine::new(cfg(vec![template_with_steps("t", vec![step("a")])]), false).unwrap();
    engine.select_template("t", None).unwrap();
    let err = engine.check(99).unwrap_err();
    assert!(matches!(err, WorkflowError::InvalidStep(_)));
    assert!(err.to_string().contains("invalid step"));
}

#[test]
fn selector_status_text_includes_issue_and_custom_prompt() {
    let config = WorkflowConfig {
        auto_continue: true,
        completion_nudge: true,
        selector_prompt: Some("Pick wisely".into()),
        templates: vec![template_with_steps("t", vec![step("a")])],
    };
    let mut engine = WorkflowEngine::new(config, false).unwrap();
    engine.set_issue(42, "fix bug".into());
    let text = engine.status_text();
    assert!(text.contains("Active issue: #42 — fix bug"));
    assert!(text.contains("Pick wisely"));
    assert!(text.contains("- t —"));
}

#[test]
fn active_status_and_prompt_render_issue_then_completion_with_guards() {
    let mut template = template_with_steps("t", vec![step("a"), step("b")]);
    template.guards = vec![WorkflowGuardRule {
        commands: vec!["git push".into()],
        before_step_key: "b".into(),
        message: "run the gate before push".into(),
    }];
    let mut engine = WorkflowEngine::new(cfg(vec![template]), true).unwrap();
    engine
        .select_template("t", Some((9, "ship it".into())))
        .unwrap();

    let status = engine.status_text();
    assert!(status.contains("Active issue: #9 — ship it"));
    let prompt = engine.prompt_snippet();
    assert!(prompt.contains("Active issue: #9 — ship it"));

    // Complete every step, then the status/prompt reflect completion + guards.
    engine.check(1).unwrap();
    engine.check(2).unwrap();
    assert_eq!(engine.mode(), WorkflowMode::Complete);
    let done = engine.prompt_snippet();
    assert!(done.contains("All workflow steps complete"));
    assert!(done.contains("run the gate before push"));
}
