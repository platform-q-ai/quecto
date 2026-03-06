use super::*;

#[test]
fn test_default_state_has_16_steps() {
    let state = WorkflowState::default_bdd();
    assert_eq!(state.steps().len(), 16);
    assert!(state.done_flags().iter().all(|&d| !d));
    assert!(state.active_issue().is_none());
}

#[test]
fn test_check_step() {
    let mut state = WorkflowState::default_bdd();
    state.check(1).unwrap();
    assert!(state.is_done(1).unwrap());
    assert!(!state.is_done(2).unwrap());
}

#[test]
fn test_uncheck_step() {
    let mut state = WorkflowState::default_bdd();
    state.check(1).unwrap();
    state.uncheck(1).unwrap();
    assert!(!state.is_done(1).unwrap());
}

#[test]
fn test_check_enforces_ordering() {
    let mut state = WorkflowState::default_bdd();
    let err = state.check(3).unwrap_err();
    assert!(err.to_string().contains("complete step 1 first"));
}

#[test]
fn test_check_allows_next_step() {
    let mut state = WorkflowState::default_bdd();
    state.check(1).unwrap();
    state.check(2).unwrap();
    assert!(state.is_done(2).unwrap());
}

#[test]
fn test_skip_bypasses_ordering() {
    let mut state = WorkflowState::default_bdd();
    state.skip(5).unwrap();
    assert!(state.is_done(5).unwrap());
    assert!(!state.is_done(4).unwrap());
}

#[test]
fn test_reset_clears_all() {
    let mut state = WorkflowState::default_bdd();
    state.check(1).unwrap();
    state.set_issue(42, "My feature".into());
    state.reset();
    assert!(state.done_flags().iter().all(|&d| !d));
    assert!(state.active_issue().is_none());
}

#[test]
fn test_set_issue() {
    let mut state = WorkflowState::default_bdd();
    state.set_issue(42, "My feature".into());
    let issue = state.active_issue().unwrap();
    assert_eq!(issue.0, 42);
    assert_eq!(issue.1, "My feature");
}

#[test]
fn test_set_issue_truncates_long_title() {
    let mut state = WorkflowState::default_bdd();
    let long_title = "x".repeat(1000);
    state.set_issue(1, long_title);
    let issue = state.active_issue().unwrap();
    assert!(issue.1.len() <= MAX_ISSUE_TITLE_LEN);
}

#[test]
fn test_clear_issue() {
    let mut state = WorkflowState::default_bdd();
    state.set_issue(42, "My feature".into());
    state.clear_issue();
    assert!(state.active_issue().is_none());
}

#[test]
fn test_progress() {
    let mut state = WorkflowState::default_bdd();
    state.check(1).unwrap();
    state.check(2).unwrap();
    let progress = state.progress();
    assert_eq!(progress.done, 2);
    assert_eq!(progress.total, 16);
    assert_eq!(progress.percent, 12);
}

#[test]
fn test_check_out_of_range() {
    let mut state = WorkflowState::default_bdd();
    let err = state.check(0).unwrap_err();
    assert!(err.to_string().contains("invalid step"));
    let err = state.check(17).unwrap_err();
    assert!(err.to_string().contains("invalid step"));
}

#[test]
fn test_uncheck_out_of_range() {
    let mut state = WorkflowState::default_bdd();
    let err = state.uncheck(0).unwrap_err();
    assert!(err.to_string().contains("invalid step"));
}

#[test]
fn test_new_clamps_steps_to_max() {
    let steps: Vec<WorkflowStep> = (0..200)
        .map(|i| WorkflowStep {
            id: i,
            label: format!("Step {}", i),
            phase: "red".into(),
        })
        .collect();
    let state = WorkflowState::new(steps);
    assert_eq!(state.steps().len(), MAX_STEPS);
}

#[test]
fn test_from_config() {
    let config = WorkflowConfig::default();
    let state = WorkflowState::from_config(&config);
    assert_eq!(state.steps().len(), 0, "default config has no steps");
}

#[test]
fn test_from_config_with_bdd_steps() {
    let config = WorkflowConfig {
        steps: bdd_steps(),
        ..Default::default()
    };
    let state = WorkflowState::from_config(&config);
    assert_eq!(state.steps().len(), 16);
}

#[test]
fn test_snapshot() {
    let mut state = WorkflowState::default_bdd();
    state.check(1).unwrap();
    state.set_issue(42, "feat".into());
    let snap = state.snapshot();
    assert_eq!(snap.steps.len(), 16);
    assert!(snap.steps[0].1); // first step done
    assert!(!snap.steps[1].1);
    assert_eq!(snap.progress.done, 1);
    assert_eq!(snap.active_issue, Some((42, "feat".into())));
}

// ─── WorkflowConfig tests ────────────────────────────────────────────────

#[test]
fn test_default_config() {
    let config = WorkflowConfig::default();
    assert!(
        !config.enabled,
        "workflow is opt-in; default should be disabled"
    );
    assert_eq!(config.steps.len(), 0, "no hardcoded default steps");
    assert!(config.guard_commit, "guard_commit should default to true");
}

#[test]
fn test_config_disabled() {
    let config = WorkflowConfig {
        enabled: false,
        ..Default::default()
    };
    assert!(!config.enabled);
}

#[test]
fn test_config_deserialize() {
    let json = r#"{"enabled":true,"steps":[{"id":1,"label":"Test","phase":"red"}]}"#;
    let config: WorkflowConfig = serde_json::from_str(json).unwrap();
    assert!(config.enabled);
    assert_eq!(config.steps.len(), 1);
}

#[test]
fn test_config_deserialize_empty() {
    let json = r#"{}"#;
    let config: WorkflowConfig = serde_json::from_str(json).unwrap();
    assert!(config.enabled);
    assert_eq!(config.steps.len(), 0, "no hardcoded default steps");
    assert!(
        config.guard_commit,
        "guard_commit defaults to true via serde"
    );
}

#[test]
fn test_config_guard_commit_false() {
    let json = r#"{"enabled":true,"guard_commit":false}"#;
    let config: WorkflowConfig = serde_json::from_str(json).unwrap();
    assert!(config.enabled);
    assert!(!config.guard_commit);
}

#[test]
fn test_config_guard_commit_default_true() {
    let json = r#"{"enabled":true}"#;
    let config: WorkflowConfig = serde_json::from_str(json).unwrap();
    assert!(config.guard_commit);
}

#[test]
fn test_config_roundtrip_skips_default_steps() {
    let config = WorkflowConfig::default();
    let json = serde_json::to_string(&config).unwrap();
    // Default steps should be skipped in serialization
    assert!(!json.contains("Update Scenarios"));
}

// ─── System prompt snippet tests ─────────────────────────────────────────

#[test]
fn test_system_prompt_snippet_with_checked_step() {
    let mut state = WorkflowState::default_bdd();
    state.check(1).unwrap();
    let snippet = state.system_prompt_snippet();
    assert!(snippet.contains("1/16"));
    assert!(snippet.contains("CURRENT STEP"));
}

#[test]
fn test_system_prompt_snippet_with_active_issue() {
    let mut state = WorkflowState::default_bdd();
    state.set_issue(42, "My feature".into());
    let snippet = state.system_prompt_snippet();
    assert!(snippet.contains("#42"));
    assert!(snippet.contains("My feature"));
}

#[test]
fn test_system_prompt_snippet_no_issue() {
    let state = WorkflowState::default_bdd();
    let snippet = state.system_prompt_snippet();
    assert!(snippet.contains("(not set)"));
}

#[test]
fn test_system_prompt_snippet_custom_steps() {
    let steps = vec![
        WorkflowStep {
            id: 1,
            label: "Custom step A".into(),
            phase: "alpha".into(),
        },
        WorkflowStep {
            id: 2,
            label: "Custom step B".into(),
            phase: "alpha".into(),
        },
        WorkflowStep {
            id: 3,
            label: "Custom step C".into(),
            phase: "beta".into(),
        },
    ];
    let state = WorkflowState::new(steps);
    let snippet = state.system_prompt_snippet();
    assert!(snippet.contains("[alpha]"));
    assert!(snippet.contains("[beta]"));
    assert!(snippet.contains("Custom step A"));
}

// ─── WorkflowError tests ─────────────────────────────────────────────────

#[test]
fn test_workflow_error_display() {
    let err = WorkflowError::InvalidStep("invalid step 0".into());
    assert_eq!(err.to_string(), "invalid step 0");
    let err = WorkflowError::OrderingViolation("complete step 1 first".into());
    assert_eq!(err.to_string(), "complete step 1 first");
}

#[test]
fn test_phase_display_name() {
    assert_eq!(phase_display_name("red"), "RED");
    assert_eq!(phase_display_name("green"), "GREEN");
    assert_eq!(phase_display_name("refactor"), "REFACTOR");
    assert_eq!(phase_display_name("ci_cd"), "CI/CD");
    assert_eq!(phase_display_name("review"), "REVIEW");
    assert_eq!(phase_display_name("custom"), "custom");
}

// ─── Auto-continue nudge tests ──────────────────────────────────────────

#[test]
fn test_auto_continue_nudge_fresh_state() {
    let state = WorkflowState::default_bdd();
    let nudge = state.auto_continue_nudge();
    assert!(nudge.is_some());
    let nudge = nudge.unwrap();
    assert!(nudge.contains("step 1"));
}

#[test]
fn test_auto_continue_nudge_partial() {
    let mut state = WorkflowState::default_bdd();
    state.check(1).unwrap();
    let nudge = state.auto_continue_nudge();
    assert!(nudge.is_some());
    let nudge = nudge.unwrap();
    assert!(nudge.contains("step 2"));
}

#[test]
fn test_auto_continue_nudge_all_done() {
    let mut state = WorkflowState::default_bdd();
    for i in 1..=16 {
        state.check(i).unwrap();
    }
    assert!(state.auto_continue_nudge().is_none());
}

// ─── Completion nudge tests ─────────────────────────────────────────────

#[test]
fn test_completion_nudge_all_done() {
    let mut state = WorkflowState::default_bdd();
    for i in 1..=16 {
        state.check(i).unwrap();
    }
    let nudge = state.completion_nudge();
    assert!(nudge.is_some());
    let nudge = nudge.unwrap();
    assert!(nudge.contains("Close"));
    assert!(nudge.contains("next"));
    assert!(nudge.contains("issue"));
}

#[test]
fn test_completion_nudge_not_done() {
    let mut state = WorkflowState::default_bdd();
    state.check(1).unwrap();
    assert!(state.completion_nudge().is_none());
}

// ─── Commit enforcement tests ───────────────────────────────────────────

#[test]
fn test_check_commit_allowed_none() {
    let state = WorkflowState::default_bdd();
    assert!(state.check_commit_allowed(None).is_ok());
}

#[test]
fn test_check_commit_allowed_zero() {
    let state = WorkflowState::default_bdd();
    assert!(state.check_commit_allowed(Some(0)).is_ok());
}

#[test]
fn test_check_commit_blocked() {
    let state = WorkflowState::default_bdd();
    let result = state.check_commit_allowed(Some(6));
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("step 1"));
}

#[test]
fn test_check_commit_allowed_when_steps_done() {
    let mut state = WorkflowState::default_bdd();
    for i in 1..=6 {
        state.check(i).unwrap();
    }
    assert!(state.check_commit_allowed(Some(6)).is_ok());
}

#[test]
fn test_check_commit_blocked_partial() {
    let mut state = WorkflowState::default_bdd();
    for i in 1..=4 {
        state.check(i).unwrap();
    }
    let result = state.check_commit_allowed(Some(6));
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("step 5"));
}

// ─── Persistence tests ──────────────────────────────────────────────────

#[test]
fn test_to_persistable() {
    let mut state = WorkflowState::default_bdd();
    state.check(1).unwrap();
    state.set_issue(42, "feat".into());
    let p = state.to_persistable();
    assert!(p.done[0]);
    assert!(!p.done[1]);
    assert_eq!(p.active_issue, Some((42, "feat".into())));
}

#[test]
fn test_from_persistable() {
    let steps = bdd_steps();
    let p = WorkflowPersistable {
        done: vec![true, false, true],
        active_issue: Some((42, "feat".into())),
    };
    let state = WorkflowState::from_persistable_with_steps(&p, Some(steps));
    assert!(state.is_done(1).unwrap());
    assert!(!state.is_done(2).unwrap());
    assert!(state.is_done(3).unwrap());
    assert_eq!(state.active_issue(), Some(&(42, "feat".into())));
}

#[test]
fn test_persistable_roundtrip() {
    let mut state = WorkflowState::default_bdd();
    state.check(1).unwrap();
    state.set_issue(42, "feat".into());
    let p = state.to_persistable();
    let json = serde_json::to_string(&p).unwrap();
    let p2: WorkflowPersistable = serde_json::from_str(&json).unwrap();
    let state2 = WorkflowState::from_persistable_with_steps(&p2, Some(bdd_steps()));
    assert!(state2.is_done(1).unwrap());
    assert!(!state2.is_done(2).unwrap());
    assert_eq!(state2.active_issue(), Some(&(42, "feat".into())));
}

#[test]
fn test_persistable_size_mismatch_pads() {
    let p = WorkflowPersistable {
        done: vec![true, false], // Only 2 items
        active_issue: None,
    };
    let state = WorkflowState::from_persistable_with_steps(&p, Some(bdd_steps()));
    assert_eq!(state.steps().len(), 16);
    assert!(state.is_done(1).unwrap());
    assert!(!state.is_done(2).unwrap());
    assert!(!state.is_done(3).unwrap()); // Padded with false
}

// ─── Config extensions tests ────────────────────────────────────────────

#[test]
fn test_default_config_has_new_fields() {
    let config = WorkflowConfig::default();
    assert!(config.auto_continue);
    assert!(config.completion_nudge);
    assert_eq!(config.enforce_commit_after_step, Some(6));
}

#[test]
fn test_config_deserialize_new_fields() {
    let json = r#"{"auto_continue":false,"completion_nudge":false,"enforce_commit_after_step":3}"#;
    let config: WorkflowConfig = serde_json::from_str(json).unwrap();
    assert!(!config.auto_continue);
    assert!(!config.completion_nudge);
    assert_eq!(config.enforce_commit_after_step, Some(3));
}

#[test]
fn test_config_deserialize_null_enforcement() {
    let json = r#"{"enforce_commit_after_step":null}"#;
    let config: WorkflowConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.enforce_commit_after_step, None);
}

#[test]
fn test_config_deserialize_missing_new_fields_uses_defaults() {
    let json = r#"{}"#;
    let config: WorkflowConfig = serde_json::from_str(json).unwrap();
    assert!(config.auto_continue);
    assert!(config.completion_nudge);
    assert_eq!(config.enforce_commit_after_step, Some(6));
}

// ─── System prompt with config tests ────────────────────────────────────

#[test]
fn test_system_prompt_snippet_with_config_enforcement() {
    let state = WorkflowState::default_bdd();
    let snippet = state.system_prompt_snippet_with_config(Some(6));
    assert!(snippet.contains("commit"));
    assert!(snippet.contains("step"));
}

#[test]
fn test_system_prompt_snippet_all_done_shows_complete() {
    let mut state = WorkflowState::default_bdd();
    for i in 1..=16 {
        state.check(i).unwrap();
    }
    let snippet = state.system_prompt_snippet();
    assert!(snippet.contains("All steps complete"));
}
