use super::*;

fn make_state(issue: Option<u32>, done: u32, total: u32) -> WorkflowBarState {
    let steps: Vec<WorkflowStepInfo> = (1..=total)
        .map(|i| WorkflowStepInfo {
            id: i,
            label: format!("Step {i} label"),
            phase: if i <= 3 {
                "red".into()
            } else if i <= 4 {
                "green".into()
            } else if i <= 6 {
                "refactor".into()
            } else if i <= 11 {
                "review".into()
            } else {
                "ci_cd".into()
            },
            done: i <= done,
        })
        .collect();
    WorkflowBarState {
        steps,
        done,
        total,
        issue_number: issue,
        issue_title: issue.map(|_| "test issue".into()),
        mode: None,
        template_name: None,
        template_count: 0,
        workflow_auto_continue: false,
        workflow_completion_nudge: false,
    }
}

#[test]
fn workflow_widget_renders_plain_text_like_quecto() {
    let mut state = make_state(Some(100), 3, 14);
    state.workflow_auto_continue = true;
    state.workflow_completion_nudge = false;
    let lines = render_widget(&state, 100);
    // main line + phase-pill overview + hints
    assert_eq!(lines.len(), 3);
    let line = &lines[0];
    assert!(
        !line.contains("\x1b[48;2;"),
        "widget should not have a full-width background: {line}"
    );
    assert!(
        line.contains("Workflow"),
        "should include widget label: {line}"
    );
    assert!(line.contains("3/14"), "should include progress: {line}");
    assert!(
        line.contains("→ Step 4"),
        "should include current step: {line}"
    );
    assert!(
        crate::interface::utils::visible_width(line) < 100,
        "widget should be content-sized, not padded to full width: {line}"
    );
    let hints = lines.last().unwrap();
    assert!(
        hints.contains("auto:on"),
        "auto toggle state missing: {hints}"
    );
    assert!(
        hints.contains("nudge:off"),
        "nudge toggle state missing: {hints}"
    );
}

#[test]
fn workflow_widget_renders_phase_pills() {
    // done=3 → all RED steps done, step 4 (GREEN) is current.
    let state = make_state(Some(100), 3, 14);
    let lines = render_widget(&state, 120);
    let pills = lines
        .iter()
        .find(|l| l.contains("RED") && l.contains("GREEN"))
        .expect("phase-pill overview line should render");
    assert!(pills.contains('✓'), "done phase marker missing: {pills}");
    assert!(pills.contains('●'), "current phase marker missing: {pills}");
    assert!(pills.contains('○'), "pending phase marker missing: {pills}");
}

#[test]
fn workflow_widget_pills_handle_custom_phase_keys() {
    // A custom-template phase key that isn't in the canonical TDD set should
    // still appear as an upper-cased pill rather than being dropped or shown as DONE.
    let mut state = make_state(Some(100), 0, 0);
    state.steps = vec![
        WorkflowStepInfo {
            id: 1,
            label: "Design".into(),
            phase: "discovery".into(),
            done: true,
        },
        WorkflowStepInfo {
            id: 2,
            label: "Build".into(),
            phase: "delivery".into(),
            done: false,
        },
    ];
    state.total = 2;
    let lines = render_widget(&state, 120);
    let pills = lines
        .iter()
        .find(|l| l.contains("DISCOVERY"))
        .expect("custom phase pill should render");
    assert!(pills.contains("DELIVERY"), "custom phase missing: {pills}");
}

#[test]
fn workflow_widget_toggle_hints_update_when_state_changes() {
    let mut state = make_state(Some(100), 3, 14);
    state.workflow_auto_continue = false;
    state.workflow_completion_nudge = true;
    let first = render_widget(&state, 100).join("\n");
    assert!(first.contains("auto:off"));
    assert!(first.contains("nudge:on"));

    state.workflow_auto_continue = true;
    state.workflow_completion_nudge = false;
    let second = render_widget(&state, 100).join("\n");
    assert!(second.contains("auto:on"));
    assert!(second.contains("nudge:off"));
}

#[test]
fn workflow_widget_hidden_when_truly_empty() {
    // Hidden only when there is genuinely no workflow: no total, no issue (#901).
    let state = make_state(None, 0, 0);
    assert!(render_widget(&state, 100).is_empty());
}

#[test]
fn workflow_widget_shown_on_selection_at_zero_of_n() {
    // Show on selection (#901): a just-selected workflow with a known total
    // renders at `0/N` before any step completes, even without an active issue.
    let state = make_state(None, 0, 14);
    let lines = render_widget(&state, 100);
    assert!(!lines.is_empty(), "0/14 with a known total must render");
    assert!(lines[0].contains("Workflow"));
    assert!(lines[0].contains("0/14"));
}

#[test]
fn workflow_widget_shown_when_issue_active() {
    let state = make_state(Some(100), 0, 14);
    let lines = render_widget(&state, 100);
    assert!(!lines.is_empty());
    assert!(lines[0].contains("Workflow"));
    assert!(lines[0].contains("0/14"));
}

#[test]
fn is_empty_only_for_no_content_bar() {
    // A `0/0`, no-steps, no-issue, no-template bar is empty regardless of mode.
    let mut empty = make_state(None, 0, 0);
    empty.mode = Some("active".into());
    assert!(empty.is_empty());
    empty.mode = Some("selecting_template".into());
    assert!(empty.is_empty());
    // Any real content makes it non-empty.
    assert!(!make_state(None, 0, 14).is_empty()); // total > 0
    assert!(!make_state(Some(5), 0, 0).is_empty()); // issue set
}

#[test]
fn signals_end_or_reset_matches_only_real_terminal_mode() {
    // The kernel emits exactly three modes (WorkflowMode::wire_str); only the
    // terminal one clears an empty bar. (#901 / finding reconciliation)
    let mut state = make_state(None, 0, 0);

    state.mode = Some("complete".into());
    assert!(state.signals_end_or_reset(), "complete is terminal");

    for transient in ["active", "selecting_template"] {
        state.mode = Some(transient.into());
        assert!(
            !state.signals_end_or_reset(),
            "{transient} is transient, must not clear"
        );
    }
    state.mode = None;
    assert!(!state.signals_end_or_reset(), "absent mode must not clear");
}

#[test]
fn workflow_widget_complete_shows_done() {
    let state = make_state(Some(100), 14, 14);
    let lines = render_widget(&state, 100);
    assert!(lines[0].contains("✓ Workflow complete!"));
}

// ── #903: empty-steps (total>0) must NOT read as complete or be visible ──────

#[test]
fn empty_steps_with_total_is_hidden_and_not_complete() {
    // Master's connect-time snapshot: total>0 (from a persisted template) but
    // done==0, no active issue, and an EMPTY steps array. Pre-#901 this was
    // hidden; it must stay hidden and must NEVER render "✓ Workflow complete!".
    let mut state = make_state(None, 0, 0);
    state.total = 14; // stale total with no steps
    assert!(
        !state.is_visible(),
        "empty-steps total>0 must not be visible"
    );
    let lines = render_widget(&state, 100);
    assert!(
        lines.is_empty(),
        "empty-steps total>0 must render nothing: {lines:?}"
    );
    assert!(
        !render_widget(&state, 100)
            .join("\n")
            .contains("Workflow complete"),
        "empty-steps must never read as complete"
    );
}

#[test]
fn empty_steps_compact_line_not_complete() {
    let mut state = make_state(None, 0, 0);
    state.total = 14;
    assert!(
        render_compact_line(&state).is_none(),
        "empty-steps total>0 must not produce a compact line"
    );
}

#[test]
fn just_selected_zero_of_n_shows_step_one_not_complete() {
    // AC#3: a genuine 0/N workflow with real steps shows on selection and
    // renders the current step (Step 1), NOT "complete". (#901 preserved.)
    let state = make_state(None, 0, 14);
    assert!(state.is_visible());
    let lines = render_widget(&state, 100);
    assert!(!lines.is_empty());
    assert!(
        lines[0].contains("→ Step 1"),
        "should show first step: {lines:?}"
    );
    assert!(
        !lines[0].contains("Workflow complete"),
        "0/N with steps must not read as complete: {lines:?}"
    );
}

#[test]
fn selecting_template_empty_steps_renders_starting_not_complete() {
    // #903: a VISIBLE-but-not-started state (selector mode, empty steps) must
    // exercise the new `"starting…"` fallback in both renderers, never the
    // spurious "✓ Workflow complete!" path.
    let mut state = make_state(None, 0, 0);
    state.mode = Some("selecting_template".into());
    assert!(state.is_visible(), "selector mode is visible");
    assert!(!state.is_complete(), "selector mode is not complete");

    let lines = render_widget(&state, 100);
    assert!(!lines.is_empty(), "selector-mode widget must render");
    assert!(
        lines[0].contains("starting"),
        "empty visible widget must render starting marker: {lines:?}"
    );
    assert!(
        !lines.join("\n").contains("Workflow complete"),
        "must never read as complete: {lines:?}"
    );

    let compact = render_compact_line(&state).expect("selector-mode compact line renders");
    assert!(
        compact.contains("starting"),
        "empty visible compact line must render starting marker: {compact}"
    );
    assert!(
        !compact.contains("Workflow complete"),
        "compact must never read as complete: {compact}"
    );
}

#[test]
fn is_complete_only_when_genuinely_done() {
    assert!(make_state(Some(1), 14, 14).is_complete(), "done==total>0");
    let mut mode_complete = make_state(None, 0, 0);
    mode_complete.mode = Some("complete".into());
    assert!(mode_complete.is_complete(), "mode==complete");
    let mut empty = make_state(None, 0, 0);
    empty.total = 14;
    assert!(!empty.is_complete(), "empty-steps done<total not complete");
    assert!(
        !make_state(Some(1), 3, 14).is_complete(),
        "mid-run not complete"
    );
}

#[test]
fn parse_workflow_event_basic() {
    let event = serde_json::json!({
        "type": "workflow_state",
        "steps": [
            {"id": 1, "label": "Scenarios", "phase": "red", "done": true},
            {"id": 2, "label": "Tests", "phase": "red", "done": false},
        ],
        "progress": {"done": 1, "total": 2, "percent": 50},
        "activeIssue": {"number": 42, "title": "test feature"},
    });
    let state = parse_workflow_event(&event);
    assert_eq!(state.done, 1);
    assert_eq!(state.total, 2);
    assert_eq!(state.issue_number, Some(42));
    assert_eq!(state.issue_title.as_deref(), Some("test feature"));
    assert_eq!(state.steps.len(), 2);
    assert!(state.steps[0].done);
    assert!(!state.steps[1].done);
}

#[test]
fn parse_workflow_event_no_issue() {
    let event = serde_json::json!({
        "type": "workflow_state",
        "steps": [],
        "progress": {"done": 0, "total": 0, "percent": 0},
    });
    let state = parse_workflow_event(&event);
    assert!(state.issue_number.is_none());
    assert!(!state.is_visible());
}

#[test]
fn parse_v2_event_captures_mode() {
    let event = serde_json::json!({
        "type": "workflow_state",
        "mode": "selecting_template",
        "availableTemplates": [{"id": 1, "label": "default"}, {"id": 2, "label": "other"}],
        "activeTemplate": {"id": 1, "label": "default"},
    });
    let state = parse_workflow_event(&event);
    assert_eq!(state.mode.as_deref(), Some("selecting_template"));
    assert_eq!(state.template_count, 2);
    assert_eq!(state.template_name.as_deref(), Some("default"));
}

#[test]
fn parse_v2_event_captures_template_name() {
    let event = serde_json::json!({
        "type": "workflow_state",
        "mode": "active",
        "activeTemplate": {"id": 1, "label": "my-template"},
        "steps": [
            {"index": 1, "label": "Scenarios", "phase": "red", "done": true},
        ],
        "progress": {"done": 1, "total": 14, "percent": 7},
    });
    let state = parse_workflow_event(&event);
    assert_eq!(state.mode.as_deref(), Some("active"));
    assert_eq!(state.template_name.as_deref(), Some("my-template"));
    assert_eq!(state.done, 1);
    assert_eq!(state.total, 14);
}

#[test]
fn parse_v2_steps_with_index_field() {
    let event = serde_json::json!({
            "type": "workflow_state",
    "steps": [{"index": 1, "label": "A", "phase": "red", "done": true}],
            "progress": {"done": 1, "total": 1, "percent": 100},
        });
    let state = parse_workflow_event(&event);
    assert_eq!(state.steps.len(), 1);
    assert_eq!(state.steps[0].id, 1);
    assert!(state.steps[0].done);
}

#[test]
fn parse_get_state_snake_case_fields() {
    let event = serde_json::json!({
        "active_issue": {"number": 99, "title": "snake case"},
        "steps": [{"id": 1, "label": "A", "phase": "red", "done": true}],
        "progress": {"done": 1, "total": 2, "percent": 50},
    });
    let state = parse_workflow_event(&event);
    assert_eq!(state.issue_number, Some(99));
    assert_eq!(state.issue_title.as_deref(), Some("snake case"));
    assert_eq!(state.done, 1);
    assert_eq!(state.total, 2);
}

#[test]
fn current_phase_returns_first_unchecked() {
    let state = make_state(Some(1), 2, 14);
    assert_eq!(state.current_phase(), Some("red"));
}

#[test]
fn current_phase_none_when_all_done() {
    let state = make_state(Some(1), 14, 14);
    assert_eq!(state.current_phase(), None);
}

#[test]
fn selector_mode_visible_even_without_issue() {
    let mut state = make_state(None, 0, 0);
    state.mode = Some("selecting_template".into());
    state.template_count = 2;
    assert!(state.is_visible());
}

// ── Compact (boxed main-pane) line: current-step context (#882) ──────────────

#[test]
fn compact_line_shows_current_step_context() {
    // done=3 of 14 → step 4 (GREEN), label "Step 4 label", issue #100.
    let state = make_state(Some(100), 3, 14);
    let line = render_compact_line(&state).expect("active workflow should render a compact line");
    let clean = crate::interface::ansi::strip_ansi(&line);
    assert!(
        clean.contains("Step 4/14"),
        "compact line must show current step number/total: {clean}"
    );
    assert!(
        clean.contains("GREEN"),
        "compact line must show current phase: {clean}"
    );
    assert!(
        clean.contains("Step 4 label"),
        "compact line must show the current step label: {clean}"
    );
    assert!(
        clean.contains("#100"),
        "compact line must show the active issue: {clean}"
    );
}

#[test]
fn compact_line_ellipsizes_long_step_label() {
    let mut state = make_state(Some(1), 0, 2);
    state.steps[0].label = "x".repeat(120);
    let line = render_compact_line(&state).expect("active workflow should render a compact line");
    let clean = crate::interface::ansi::strip_ansi(&line);
    assert!(
        clean.contains('…'),
        "an over-long step label must be ellipsized: {clean}"
    );
    assert!(
        !clean.contains(&"x".repeat(120)),
        "the full over-long label must not be rendered verbatim: {clean}"
    );
}

#[test]
fn compact_line_complete_state_has_no_misleading_step() {
    let state = make_state(Some(7), 14, 14);
    let line = render_compact_line(&state).expect("a complete workflow is still visible");
    let clean = crate::interface::ansi::strip_ansi(&line);
    assert!(
        !clean.contains("Step "),
        "a complete workflow must not show a current step: {clean}"
    );
    assert!(
        clean.to_lowercase().contains("complete"),
        "a complete workflow should read as complete: {clean}"
    );
}

#[test]
fn compact_line_exposes_auto_continue_state() {
    // #897 AC2: the always-visible main-pane line must surface auto_continue so
    // its overriding of "wait"-style instructions is never surprising.
    let mut state = make_state(Some(100), 3, 14);
    state.workflow_auto_continue = true;
    let on = render_compact_line(&state).expect("active workflow renders a compact line");
    let on_clean = crate::interface::ansi::strip_ansi(&on).to_lowercase();
    assert!(
        on_clean.contains("auto:on"),
        "compact line must show auto-continue ON: {on_clean}"
    );

    state.workflow_auto_continue = false;
    let off = render_compact_line(&state).expect("active workflow renders a compact line");
    let off_clean = crate::interface::ansi::strip_ansi(&off).to_lowercase();
    assert!(
        off_clean.contains("auto:off"),
        "compact line must show auto-continue OFF: {off_clean}"
    );
}

#[test]
fn compact_line_none_when_inactive() {
    let state = make_state(None, 0, 0);
    assert!(
        render_compact_line(&state).is_none(),
        "no compact line should render when no workflow is active"
    );
}
