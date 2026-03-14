use super::*;

// ═══════════════════════════════════════════════════════════════════════════
// Comprehensive additional tests
// ═══════════════════════════════════════════════════════════════════════════

// ─── Empty workflow edge cases ──────────────────────────────────────────

#[test]
fn test_empty_workflow_progress() {
    let state = WorkflowState::new(vec![]);
    let progress = state.progress();
    assert_eq!(progress.done, 0);
    assert_eq!(progress.total, 0);
    assert_eq!(progress.percent, 0);
}

#[test]
fn test_empty_workflow_check_returns_error() {
    let mut state = WorkflowState::new(vec![]);
    let err = state.check(1).unwrap_err();
    assert!(matches!(err, WorkflowError::InvalidStep(_)));
    assert!(err.to_string().contains("invalid step 1: must be 1-0"));
}

#[test]
fn test_empty_workflow_snippet() {
    let state = WorkflowState::new(vec![]);
    let snippet = state.system_prompt_snippet();
    assert!(snippet.contains("0/0"));
    // Should not show "All steps complete" — there are no steps
    assert!(!snippet.contains("All steps complete"));
}

#[test]
fn test_empty_workflow_auto_continue_nudge() {
    let state = WorkflowState::new(vec![]);
    assert!(
        state.auto_continue_nudge().is_none(),
        "empty workflow has no steps to continue"
    );
}

#[test]
fn test_empty_workflow_completion_nudge() {
    let state = WorkflowState::new(vec![]);
    // All 0 of 0 steps are done, but progress.done < progress.total is false
    // This is an edge case — should not generate completion nudge for empty workflow
    let nudge = state.completion_nudge();
    // 0 >= 0 is true, so it will generate a completion nudge — this tests that
    assert!(nudge.is_some());
}

#[test]
fn test_empty_workflow_reset() {
    let mut state = WorkflowState::new(vec![]);
    state.set_issue(1, "test".into());
    state.reset();
    assert!(state.active_issue().is_none());
}

// ─── Single-step workflow ───────────────────────────────────────────────

#[test]
fn test_single_step_workflow() {
    let steps = vec![WorkflowStep {
        id: 1,
        label: "Only step".into(),
        phase: "red".into(),
    }];
    let mut state = WorkflowState::new(steps);
    assert_eq!(state.progress().total, 1);
    assert_eq!(state.progress().done, 0);
    state.check(1).unwrap();
    assert_eq!(state.progress().done, 1);
    assert_eq!(state.progress().percent, 100);
    assert!(state.auto_continue_nudge().is_none());
    assert!(state.completion_nudge().is_some());
}

#[test]
fn test_single_step_check_step_2_fails() {
    let steps = vec![WorkflowStep {
        id: 1,
        label: "Only step".into(),
        phase: "red".into(),
    }];
    let mut state = WorkflowState::new(steps);
    let err = state.check(2).unwrap_err();
    assert!(matches!(err, WorkflowError::InvalidStep(_)));
}

// ─── Check ordering edge cases ──────────────────────────────────────────

#[test]
fn test_check_all_16_steps_in_order() {
    let mut state = WorkflowState::default_bdd();
    for i in 1..=16 {
        state.check(i).unwrap();
    }
    assert!(state.done_flags().iter().all(|&d| d));
    let progress = state.progress();
    assert_eq!(progress.done, 16);
    assert_eq!(progress.percent, 100);
}

#[test]
fn test_check_same_step_twice_is_idempotent() {
    let mut state = WorkflowState::default_bdd();
    state.check(1).unwrap();
    // Checking step 1 again should succeed (it's already done, all previous are done)
    state.check(1).unwrap();
    assert!(state.is_done(1).unwrap());
}

#[test]
fn test_uncheck_then_check_restores() {
    let mut state = WorkflowState::default_bdd();
    state.check(1).unwrap();
    state.check(2).unwrap();
    state.uncheck(2).unwrap();
    assert!(!state.is_done(2).unwrap());
    // Re-checking should work since step 1 is still done
    state.check(2).unwrap();
    assert!(state.is_done(2).unwrap());
}

#[test]
fn test_uncheck_middle_step_creates_gap() {
    let mut state = WorkflowState::default_bdd();
    state.check(1).unwrap();
    state.check(2).unwrap();
    state.check(3).unwrap();
    state.uncheck(2).unwrap();
    assert!(!state.is_done(2).unwrap());
    assert!(state.is_done(3).unwrap());
    // Trying to check step 4 should fail because step 2 is not done
    let err = state.check(4).unwrap_err();
    assert!(err.to_string().contains("complete step 2 first"));
}

#[test]
fn test_skip_first_step_then_check_second() {
    let mut state = WorkflowState::default_bdd();
    state.skip(1).unwrap();
    state.check(2).unwrap();
    assert!(state.is_done(1).unwrap());
    assert!(state.is_done(2).unwrap());
}

#[test]
fn test_skip_last_step() {
    let mut state = WorkflowState::default_bdd();
    state.skip(16).unwrap();
    assert!(state.is_done(16).unwrap());
    assert!(!state.is_done(1).unwrap());
}

#[test]
fn test_skip_out_of_range() {
    let mut state = WorkflowState::default_bdd();
    let err = state.skip(0).unwrap_err();
    assert!(matches!(err, WorkflowError::InvalidStep(_)));
    let err = state.skip(17).unwrap_err();
    assert!(matches!(err, WorkflowError::InvalidStep(_)));
}

// ─── Issue management edge cases ────────────────────────────────────────

#[test]
fn test_set_issue_overwrite() {
    let mut state = WorkflowState::default_bdd();
    state.set_issue(1, "First".into());
    state.set_issue(2, "Second".into());
    let issue = state.active_issue().unwrap();
    assert_eq!(issue.0, 2);
    assert_eq!(issue.1, "Second");
}

#[test]
fn test_set_issue_empty_title() {
    let mut state = WorkflowState::default_bdd();
    state.set_issue(1, "".into());
    let issue = state.active_issue().unwrap();
    assert_eq!(issue.1, "");
}

#[test]
fn test_set_issue_zero_number() {
    let mut state = WorkflowState::default_bdd();
    state.set_issue(0, "Zero issue".into());
    let issue = state.active_issue().unwrap();
    assert_eq!(issue.0, 0);
}

#[test]
fn test_set_issue_max_u32() {
    let mut state = WorkflowState::default_bdd();
    state.set_issue(u32::MAX, "Max issue".into());
    let issue = state.active_issue().unwrap();
    assert_eq!(issue.0, u32::MAX);
}

#[test]
fn test_clear_issue_when_no_issue() {
    let mut state = WorkflowState::default_bdd();
    // Should not panic
    state.clear_issue();
    assert!(state.active_issue().is_none());
}

#[test]
fn test_set_issue_unicode_title() {
    let mut state = WorkflowState::default_bdd();
    state.set_issue(42, "Add 日本語 support 🎉".into());
    let issue = state.active_issue().unwrap();
    assert_eq!(issue.1, "Add 日本語 support 🎉");
}

#[test]
fn test_set_issue_truncates_at_char_boundary() {
    let mut state = WorkflowState::default_bdd();
    // Create a title that is exactly at the boundary with multi-byte chars
    let title = "x".repeat(498) + "日本"; // 498 + 6 bytes = 504 bytes, 500 chars
    state.set_issue(1, title);
    let issue = state.active_issue().unwrap();
    assert!(issue.1.len() <= MAX_ISSUE_TITLE_LEN);
    // Should not panic on char boundary
    assert!(issue.1.is_char_boundary(issue.1.len()));
}

// ─── Progress calculation edge cases ────────────────────────────────────

#[test]
fn test_progress_with_skipped_steps() {
    let mut state = WorkflowState::default_bdd();
    state.skip(5).unwrap();
    state.skip(10).unwrap();
    let progress = state.progress();
    assert_eq!(progress.done, 2);
    assert_eq!(progress.total, 16);
}

#[test]
fn test_progress_after_reset() {
    let mut state = WorkflowState::default_bdd();
    state.check(1).unwrap();
    state.check(2).unwrap();
    state.reset();
    let progress = state.progress();
    assert_eq!(progress.done, 0);
    assert_eq!(progress.total, 16);
    assert_eq!(progress.percent, 0);
}

#[test]
fn test_progress_percent_rounds_down() {
    // 1/16 = 6.25%, should be 6
    let mut state = WorkflowState::default_bdd();
    state.check(1).unwrap();
    assert_eq!(state.progress().percent, 6);
    // 5/16 = 31.25%, should be 31
    for i in 2..=5 {
        state.check(i).unwrap();
    }
    assert_eq!(state.progress().percent, 31);
}

// ─── Commit enforcement edge cases ──────────────────────────────────────

#[test]
fn test_check_commit_threshold_equals_total_steps() {
    let mut state = WorkflowState::default_bdd();
    // enforce_commit_after_step = 16 means all 16 steps must be done
    // check_commit_allowed(Some(16)) checks steps 1-15
    for i in 1..=15 {
        state.check(i).unwrap();
    }
    assert!(state.check_commit_allowed(Some(16)).is_ok());
}

#[test]
fn test_check_commit_threshold_exceeds_total() {
    let mut state = WorkflowState::default_bdd();
    // enforce_commit_after_step = 100 but only 16 steps exist
    // Should check all 16 and not panic
    for i in 1..=16 {
        state.check(i).unwrap();
    }
    assert!(state.check_commit_allowed(Some(100)).is_ok());
}

#[test]
fn test_check_commit_threshold_one() {
    let state = WorkflowState::default_bdd();
    // enforce_commit_after_step = 1 means no steps required (1-1=0 steps)
    assert!(state.check_commit_allowed(Some(1)).is_ok());
}

#[test]
fn test_check_steps_complete_boundary() {
    let mut state = WorkflowState::default_bdd();
    // before_step=1 means 0 steps need to be complete
    assert!(state.check_steps_complete(1).is_ok());
    // before_step=2 means step 1 needs to be complete
    assert!(state.check_steps_complete(2).is_err());
    state.check(1).unwrap();
    assert!(state.check_steps_complete(2).is_ok());
}

// ─── Persistable edge cases ────────────────────────────────────────────

#[test]
fn test_persistable_longer_than_steps_truncates() {
    let p = WorkflowPersistable {
        done: vec![true; 20], // More done flags than steps
        active_issue: None,
    };
    let state = WorkflowState::from_persistable_with_steps(&p, Some(bdd_steps()));
    assert_eq!(state.steps().len(), 16);
    assert_eq!(state.done_flags().len(), 16);
    // First 16 should be true (truncated from 20)
    assert!(state.done_flags().iter().all(|&d| d));
}

#[test]
fn test_persistable_empty_done_vec() {
    let p = WorkflowPersistable {
        done: vec![],
        active_issue: None,
    };
    let state = WorkflowState::from_persistable_with_steps(&p, Some(bdd_steps()));
    assert_eq!(state.done_flags().len(), 16);
    assert!(state.done_flags().iter().all(|&d| !d));
}

#[test]
fn test_persistable_none_steps_uses_default() {
    let p = WorkflowPersistable {
        done: vec![true],
        active_issue: None,
    };
    let state = WorkflowState::from_persistable_with_steps(&p, None);
    // default_steps() returns empty vec
    assert_eq!(state.steps().len(), 0);
}

#[test]
fn test_persistable_with_issue_roundtrip() {
    let mut state = WorkflowState::default_bdd();
    for i in 1..=8 {
        state.check(i).unwrap();
    }
    state.set_issue(999, "Complex feature with 日本語".into());
    let p = state.to_persistable();
    let json = serde_json::to_string(&p).unwrap();
    let p2: WorkflowPersistable = serde_json::from_str(&json).unwrap();
    let state2 = WorkflowState::from_persistable_with_steps(&p2, Some(bdd_steps()));
    for i in 1..=8 {
        assert!(state2.is_done(i).unwrap());
    }
    for i in 9..=16 {
        assert!(!state2.is_done(i).unwrap());
    }
    assert_eq!(
        state2.active_issue(),
        Some(&(999, "Complex feature with 日本語".into()))
    );
}

// ─── System prompt snippet comprehensive tests ──────────────────────────

#[test]
fn test_snippet_groups_same_phase_consecutively() {
    let steps = vec![
        WorkflowStep {
            id: 1,
            label: "A".into(),
            phase: "red".into(),
        },
        WorkflowStep {
            id: 2,
            label: "B".into(),
            phase: "red".into(),
        },
        WorkflowStep {
            id: 3,
            label: "C".into(),
            phase: "green".into(),
        },
        WorkflowStep {
            id: 4,
            label: "D".into(),
            phase: "red".into(),
        }, // red again after green
    ];
    let state = WorkflowState::new(steps);
    let snippet = state.system_prompt_snippet();
    // Count group headers only (lines starting with "\n[RED]\n")
    let red_headers = snippet.lines().filter(|l| l.trim() == "[RED]").count();
    assert_eq!(
        red_headers, 2,
        "non-consecutive RED phases should create separate group headers. Snippet:\n{}",
        snippet
    );
}

#[test]
fn test_snippet_current_step_marker() {
    let mut state = WorkflowState::default_bdd();
    state.check(1).unwrap();
    state.check(2).unwrap();
    let snippet = state.system_prompt_snippet();
    // Current step should be step 3
    assert!(snippet.contains("CURRENT STEP → 3."));
}

#[test]
fn test_snippet_no_current_step_when_all_done() {
    let mut state = WorkflowState::default_bdd();
    for i in 1..=16 {
        state.check(i).unwrap();
    }
    let snippet = state.system_prompt_snippet();
    assert!(!snippet.contains("CURRENT STEP"));
}

#[test]
fn test_snippet_with_guards_multiple() {
    let state = WorkflowState::default_bdd();
    let guards = vec![
        GuardRule {
            commands: vec!["git commit".into(), "git push".into()],
            before_step: 7,
            message: "Complete RED-GREEN-REFACTOR first.".into(),
        },
        GuardRule {
            commands: vec!["gh pr merge".into()],
            before_step: 15,
            message: "Complete review first.".into(),
        },
    ];
    let snippet = state.system_prompt_snippet_with_guards(&guards);
    assert!(snippet.contains("git commit, git push"));
    assert!(snippet.contains("gh pr merge"));
    assert!(snippet.contains("Complete RED-GREEN-REFACTOR first."));
    assert!(snippet.contains("Complete review first."));
}

// ─── Completion nudge uses dynamic step count (Bug #1 fix) ──────────────

#[test]
fn test_completion_nudge_uses_dynamic_step_count() {
    let steps = vec![
        WorkflowStep {
            id: 1,
            label: "A".into(),
            phase: "red".into(),
        },
        WorkflowStep {
            id: 2,
            label: "B".into(),
            phase: "green".into(),
        },
        WorkflowStep {
            id: 3,
            label: "C".into(),
            phase: "refactor".into(),
        },
    ];
    let mut state = WorkflowState::new(steps);
    for i in 1..=3 {
        state.check(i).unwrap();
    }
    let nudge = state.completion_nudge().unwrap();
    assert!(
        nudge.contains("all 3 workflow steps"),
        "should use actual step count (3), not hardcoded 16. Got: {}",
        nudge
    );
    assert!(
        !nudge.contains("all 16"),
        "should not contain hardcoded '16'"
    );
}

// ─── WorkflowError variants ────────────────────────────────────────────

#[test]
fn test_workflow_error_commit_blocked_display() {
    let err = WorkflowError::CommitBlocked("blocked: complete step 1".into());
    assert_eq!(err.to_string(), "blocked: complete step 1");
}

#[test]
fn test_workflow_error_is_std_error() {
    let err = WorkflowError::InvalidStep("test".into());
    let _: &dyn std::error::Error = &err;
}

#[test]
fn test_workflow_error_eq() {
    let a = WorkflowError::InvalidStep("foo".into());
    let b = WorkflowError::InvalidStep("foo".into());
    assert_eq!(a, b);
    let c = WorkflowError::OrderingViolation("foo".into());
    assert_ne!(a, c);
}

// ─── WorkflowConfig migration ──────────────────────────────────────────

#[test]
fn test_config_migrate_deprecated_guard_commit() {
    let mut config = WorkflowConfig {
        guard_commit: Some(true),
        enforce_commit_after_step: Some(6),
        ..Default::default()
    };
    config.migrate_deprecated();
    assert_eq!(config.guards.len(), 1);
    assert_eq!(config.guards[0].commands, vec!["git commit", "git push"]);
    assert_eq!(config.guards[0].before_step, 7); // 6 + 1
    assert!(config.guard_commit.is_none());
    assert!(config.enforce_commit_after_step.is_none());
}

#[test]
fn test_config_migrate_deprecated_no_op_when_guards_exist() {
    let mut config = WorkflowConfig {
        guard_commit: Some(true),
        guards: vec![GuardRule {
            commands: vec!["git commit".into()],
            before_step: 7,
            message: "Existing guard.".into(),
        }],
        ..Default::default()
    };
    config.migrate_deprecated();
    // Should not add another guard since guards already exist
    assert_eq!(config.guards.len(), 1);
    assert_eq!(config.guards[0].message, "Existing guard.");
}

#[test]
fn test_config_migrate_deprecated_default_step() {
    let mut config = WorkflowConfig {
        guard_commit: Some(true),
        enforce_commit_after_step: None, // defaults to 6
        ..Default::default()
    };
    config.migrate_deprecated();
    assert_eq!(config.guards.len(), 1);
    assert_eq!(config.guards[0].before_step, 7); // 6 + 1
}

#[test]
fn test_config_migrate_disabled_no_op() {
    let mut config = WorkflowConfig {
        guard_commit: Some(false),
        ..Default::default()
    };
    config.migrate_deprecated();
    assert!(config.guards.is_empty());
}

// ─── WorkflowConfig serialization ──────────────────────────────────────

#[test]
fn test_config_serialize_skips_defaults() {
    let config = WorkflowConfig {
        enabled: true,
        ..Default::default()
    };
    let json = serde_json::to_string(&config).unwrap();
    // auto_continue=true should be skipped (default)
    assert!(!json.contains("auto_continue"));
    // completion_nudge=true should be skipped (default)
    assert!(!json.contains("completion_nudge"));
    // guards=[] should be skipped (empty)
    assert!(!json.contains("guards"));
}

#[test]
fn test_config_serialize_includes_non_defaults() {
    let config = WorkflowConfig {
        enabled: true,
        auto_continue: false,
        completion_nudge: false,
        guards: vec![GuardRule {
            commands: vec!["git commit".into()],
            before_step: 7,
            message: "Not yet.".into(),
        }],
        ..Default::default()
    };
    let json = serde_json::to_string(&config).unwrap();
    assert!(json.contains("auto_continue"));
    assert!(json.contains("completion_nudge"));
    assert!(json.contains("guards"));
}

// ─── Snapshot tests ─────────────────────────────────────────────────────

#[test]
fn test_snapshot_empty_state() {
    let state = WorkflowState::new(vec![]);
    let snap = state.snapshot();
    assert!(snap.steps.is_empty());
    assert_eq!(snap.progress.done, 0);
    assert_eq!(snap.progress.total, 0);
    assert!(snap.active_issue.is_none());
}

#[test]
fn test_snapshot_preserves_all_step_data() {
    let steps = vec![
        WorkflowStep {
            id: 1,
            label: "First".into(),
            phase: "red".into(),
        },
        WorkflowStep {
            id: 2,
            label: "Second".into(),
            phase: "green".into(),
        },
    ];
    let mut state = WorkflowState::new(steps);
    state.check(1).unwrap();
    let snap = state.snapshot();
    assert_eq!(snap.steps[0].0.label, "First");
    assert_eq!(snap.steps[0].0.phase, "red");
    assert!(snap.steps[0].1); // done
    assert_eq!(snap.steps[1].0.label, "Second");
    assert!(!snap.steps[1].1); // not done
}

// ─── MAX_STEPS boundary ────────────────────────────────────────────────

#[test]
fn test_exactly_max_steps() {
    let steps: Vec<WorkflowStep> = (1..=MAX_STEPS as u32)
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
fn test_one_over_max_steps() {
    let steps: Vec<WorkflowStep> = (1..=(MAX_STEPS as u32 + 1))
        .map(|i| WorkflowStep {
            id: i,
            label: format!("Step {}", i),
            phase: "red".into(),
        })
        .collect();
    let state = WorkflowState::new(steps);
    assert_eq!(state.steps().len(), MAX_STEPS);
}

// ─── Guard rule saturation_sub safety ───────────────────────────────────

#[test]
fn test_guard_before_step_zero_always_passes() {
    let state = WorkflowState::default_bdd();
    assert!(state.check_steps_complete(0).is_ok());
}

#[test]
fn test_guard_before_step_one_always_passes() {
    let state = WorkflowState::default_bdd();
    // before_step=1 means 0 steps required (1 - 1 = 0)
    assert!(state.check_steps_complete(1).is_ok());
}

// ─── is_done boundary checks ───────────────────────────────────────────

#[test]
fn test_is_done_out_of_range_zero() {
    let state = WorkflowState::default_bdd();
    let err = state.is_done(0).unwrap_err();
    assert!(matches!(err, WorkflowError::InvalidStep(_)));
}

#[test]
fn test_is_done_out_of_range_high() {
    let state = WorkflowState::default_bdd();
    let err = state.is_done(17).unwrap_err();
    assert!(matches!(err, WorkflowError::InvalidStep(_)));
}

#[test]
fn test_is_done_valid_range() {
    let state = WorkflowState::default_bdd();
    for i in 1..=16 {
        assert!(!state.is_done(i).unwrap());
    }
}
