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
    assert_eq!(
        ids,
        [
            "feature",
            "adversarial-review",
            "bugfix",
            "chore",
            "flake-hunt",
            "investigate",
            "plan",
            "prd",
            "refactor",
            "remove",
        ]
    );
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
    assert_eq!(snap.progress.total, 20);
    assert_eq!(snap.current_step.unwrap().key, "hooks");
    assert_eq!(snap.steps.len(), 20);
    assert!(snap.guards_enabled);
}

#[test]
fn selector_status_mentions_select_template() {
    let engine = WorkflowEngine::new(WorkflowConfig::default(), false).unwrap();
    let status = engine.status_text();
    assert!(status.contains("select_template"));
    assert!(status.contains("feature"));
}

#[test]
fn active_status_mentions_guidance() {
    let mut engine = WorkflowEngine::new(WorkflowConfig::default(), false).unwrap();
    engine.select_template("feature", None).unwrap();
    engine.check(1).unwrap();
    let status = engine.status_text();
    assert!(status.contains("CURRENT STEP"));
    assert!(status.contains("acceptance criteria"));
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

// ── #1113 cache-safe prompting: idle-boundary nudges carry workflow state ───

/// Single-step template with a distinctive, non-dictionary id so selector
/// assertions cannot pass on prose that merely mentions e.g. "feature".
fn probe_template(id: &str, label: &str) -> WorkflowTemplate {
    WorkflowTemplate {
        id: id.into(),
        label: label.into(),
        description: "probe template".into(),
        when_to_use: None,
        steps: vec![WorkflowTemplateStep {
            key: "only".into(),
            label: "Only probe step".into(),
            phase: "red".into(),
            guidance: None,
        }],
        guards: vec![],
    }
}

fn nudge_probe_config(templates: Vec<WorkflowTemplate>) -> WorkflowConfig {
    WorkflowConfig {
        auto_continue: true,
        templates,
        ..WorkflowConfig::default()
    }
}

/// #1113 AC4: with a static system prompt, the auto-continue nudge is the
/// idle-boundary channel for the current step — it must carry the step's
/// label and its guidance blob.
#[test]
fn auto_continue_nudge_carries_current_step_label_and_guidance() {
    let mut template = probe_template("t", "T");
    template.steps[0].label = "Alpha planning step".into();
    template.steps[0].guidance = Some("guidance for step alpha".into());
    let mut engine = WorkflowEngine::new(nudge_probe_config(vec![template]), false).unwrap();
    engine.select_template("t", None).unwrap();

    let nudge = engine
        .auto_continue_nudge()
        .expect("active incomplete workflow yields a nudge");
    assert!(
        nudge.contains("Alpha planning step"),
        "nudge must carry the current step label: {nudge}"
    );
    assert!(
        nudge.contains("guidance for step alpha"),
        "nudge must carry the current step guidance: {nudge}"
    );
}

/// #1113 AC4: the corrective idle-boundary variant (sent after a stalled
/// nudged turn) must carry the current step's label and guidance too.
#[test]
fn corrective_nudge_carries_current_step_label_and_guidance() {
    let mut template = probe_template("t", "T");
    template.steps[0].label = "Alpha planning step".into();
    template.steps[0].guidance = Some("guidance for step alpha".into());
    let mut engine = WorkflowEngine::new(nudge_probe_config(vec![template]), false).unwrap();
    engine.select_template("t", None).unwrap();

    let nudge = engine
        .corrective_nudge()
        .expect("active incomplete workflow yields a corrective nudge");
    assert!(
        nudge.contains("Alpha planning step"),
        "corrective nudge must carry the current step label: {nudge}"
    );
    assert!(
        nudge.contains("guidance for step alpha"),
        "corrective nudge must carry the current step guidance: {nudge}"
    );
}

/// #1113 AC3: an explicit `--workflow` session (selector nudge armed) with no
/// template selected must push the template selector — listing the actual
/// templates — through BOTH nudge wordings: the dispatch loop
/// (`workflow_nudge_message`) requires standard AND corrective to be `Some`,
/// so extending only one would silently never deliver the selector.
#[test]
fn idle_nudges_present_template_selector_before_selection() {
    let templates = vec![
        probe_template("qx-selector-probe", "QX Selector Probe"),
        probe_template("qx-other-probe", "QX Other Probe"),
    ];
    let mut engine = WorkflowEngine::new(nudge_probe_config(templates), false).unwrap();
    engine.set_selector_nudge(true);
    assert_eq!(engine.mode(), WorkflowMode::SelectingTemplate);

    for (variant, nudge) in [
        ("standard", engine.auto_continue_nudge()),
        ("corrective", engine.corrective_nudge()),
    ] {
        let nudge = nudge
            .as_deref()
            .expect("selector-mode idle boundary must push the selector nudge");
        assert!(
            nudge.contains("select_template"),
            "{variant} selector nudge must instruct selection via select_template: {nudge}"
        );
        for (id, label) in [
            ("qx-selector-probe", "QX Selector Probe"),
            ("qx-other-probe", "QX Other Probe"),
        ] {
            assert!(
                nudge.contains(id) && nudge.contains(label),
                "{variant} selector nudge must list template '{id}' ({label}): {nudge}"
            );
        }
    }
}

/// #1113: the idle-boundary selector nudge is the SOLE proactive selection
/// channel, so it must carry everything the retired system-prompt selector
/// carried — the operator-configured `workflow.selector_prompt` and the
/// active issue. Otherwise the config knob is silently dead in the exact
/// flow (`--workflow` start-up) it was designed for.
#[test]
fn selector_nudge_carries_selector_prompt_and_active_issue() {
    let mut config = nudge_probe_config(vec![probe_template("qx-selector-probe", "QX Probe")]);
    config.selector_prompt =
        Some("Operator rule: for production incidents always choose hotfix".into());
    let mut engine = WorkflowEngine::new(config, false).unwrap();
    engine.set_selector_nudge(true);
    engine.set_issue(42, "Fix the flaky gate".to_string());

    for (variant, nudge) in [
        ("standard", engine.auto_continue_nudge()),
        ("corrective", engine.corrective_nudge()),
    ] {
        let nudge = nudge
            .as_deref()
            .expect("selector-mode idle boundary must push the selector nudge");
        assert!(
            nudge.contains("Operator rule: for production incidents always choose hotfix"),
            "{variant} selector nudge must carry the configured selector_prompt: {nudge}"
        );
        assert!(
            nudge.contains("#42") && nudge.contains("Fix the flaky gate"),
            "{variant} selector nudge must carry the active issue: {nudge}"
        );
    }
}

/// #1113: the selector nudge is armed only for explicit `--workflow`
/// sessions. A plain UDS session (workflow tool available, nothing armed)
/// must never be nudged to pick a template at idle boundaries.
#[test]
fn selector_nudge_requires_explicit_arming() {
    let engine = WorkflowEngine::new(WorkflowConfig::default(), false).unwrap();
    assert_eq!(engine.mode(), WorkflowMode::SelectingTemplate);
    assert!(
        engine.auto_continue_nudge().is_none(),
        "unarmed selector mode must not yield an auto-continue nudge"
    );
    assert!(
        engine.corrective_nudge().is_none(),
        "unarmed selector mode must not yield a corrective nudge"
    );
}

/// #1113 AC3 regression: the selector nudge must NOT be gated on
/// `workflow.auto_continue`. The retired system-prompt selector reached the
/// model regardless of that setting, and the idle-boundary nudge is now the
/// sole proactive selection channel — with auto-continue disabled, an armed
/// unselected session must still be told to select a template, while
/// active-step continuation stays gated on auto-continue.
#[test]
fn selector_nudge_fires_with_auto_continue_disabled() {
    let mut config = nudge_probe_config(vec![probe_template("qx-selector-probe", "QX Probe")]);
    config.auto_continue = false;
    let mut engine = WorkflowEngine::new(config, false).unwrap();
    engine.set_selector_nudge(true);

    for (variant, nudge) in [
        ("standard", engine.auto_continue_nudge()),
        ("corrective", engine.corrective_nudge()),
    ] {
        let nudge = nudge
            .as_deref()
            .expect("selector nudge must fire with auto-continue disabled");
        assert!(
            nudge.contains("select_template") && nudge.contains("qx-selector-probe"),
            "{variant} selector nudge must present the selector: {nudge}"
        );
    }

    // Once a template is active, step continuation is still auto-continue's.
    engine.select_template("qx-selector-probe", None).unwrap();
    assert!(
        engine.auto_continue_nudge().is_none() && engine.corrective_nudge().is_none(),
        "active-step nudges must stay gated on auto-continue"
    );
}

/// #1113 AC2: the tool-result step handoff must carry the progress count and
/// the active issue alongside the step focus — the retired per-turn system
/// prompt carried all three, and the tool result is its immediate
/// replacement channel after `select_template`/`check`/`skip`/`uncheck`.
#[test]
fn step_handoff_text_carries_progress_and_active_issue() {
    let mut template = probe_template("qx-handoff-probe", "QX Handoff Probe");
    template.steps.push(WorkflowTemplateStep {
        key: "second".into(),
        label: "Second probe step".into(),
        phase: "green".into(),
        guidance: None,
    });
    let mut engine = WorkflowEngine::new(nudge_probe_config(vec![template]), false).unwrap();
    engine
        .select_template("qx-handoff-probe", Some((77, "Handoff probe issue".into())))
        .unwrap();
    engine.check(1).unwrap();

    let handoff = engine.step_handoff_text("Next step");
    assert!(
        handoff.contains("Second probe step"),
        "handoff must carry the current step focus: {handoff}"
    );
    assert!(
        handoff.contains("Progress: 1/2 steps complete."),
        "handoff must carry the progress count: {handoff}"
    );
    assert!(
        handoff.contains("#77") && handoff.contains("Handoff probe issue"),
        "handoff must carry the active issue: {handoff}"
    );
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
