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
fn hidden_when_no_issue() {
    let state = make_state(None, 0, 14);
    assert!(render(&state, 80).is_empty());
}

#[test]
fn visible_with_issue() {
    let state = make_state(Some(559), 4, 14);
    let lines = render(&state, 80);
    assert_eq!(lines.len(), 4, "blank + content + stage/tips + blank");
}

#[test]
fn contains_issue_number() {
    let state = make_state(Some(559), 4, 14);
    let line = &render(&state, 80)[1];
    assert!(
        line.contains("559"),
        "should contain issue number: {}",
        line
    );
}

#[test]
fn contains_progress_fraction() {
    let state = make_state(Some(100), 7, 14);
    let line = &render(&state, 80)[1];
    assert!(line.contains("07/14"), "should contain progress: {}", line);
}

#[test]
fn contains_phase_name() {
    let state = make_state(Some(100), 3, 14);
    let line = &render(&state, 80)[1];
    assert!(line.contains("GREEN"), "step 4 is green phase: {}", line);
}

#[test]
fn contains_progress_bar_chars() {
    let state = make_state(Some(100), 7, 14);
    let line = &render(&state, 80)[1];
    assert!(line.contains('▓'), "should contain filled block");
    assert!(line.contains('░'), "should contain empty block");
}

#[test]
fn contains_box_drawing() {
    let state = make_state(Some(100), 4, 14);
    let line = &render(&state, 80)[1];
    assert!(line.contains('─'), "should contain box drawing dash");
    assert!(line.contains('▐'), "should contain right half block sigil");
}

#[test]
fn contains_angle_brackets() {
    let state = make_state(Some(100), 4, 14);
    let line = &render(&state, 80)[1];
    assert!(line.contains('⟨'), "should contain left angle bracket");
    assert!(line.contains('⟩'), "should contain right angle bracket");
}

#[test]
fn contains_true_colour_bg() {
    let state = make_state(Some(100), 0, 14);
    let line = &render(&state, 80)[1];
    // RED phase bg
    assert!(
        line.contains("\x1b[48;2;"),
        "should contain true-colour bg escape"
    );
}

#[test]
fn renders_stage_status_with_hotkey_tips() {
    let mut state = make_state(Some(100), 3, 14);
    state.workflow_auto_continue = true;
    state.workflow_completion_nudge = false;
    let lines = render(&state, 120);
    let stage_line = lines
        .iter()
        .find(|line| line.contains("RED") && line.contains("GREEN"))
        .expect("stage status line should render");
    assert!(
        stage_line.contains("●") && stage_line.contains("GREEN"),
        "current phase should be marked: {stage_line}"
    );
    assert!(
        stage_line.contains("✓") && stage_line.contains("RED"),
        "done stages should be marked: {stage_line}"
    );
    assert!(
        stage_line.contains("A:auto on"),
        "auto-continue status missing: {stage_line}"
    );
    assert!(
        stage_line.contains("N:nudge off"),
        "nudge status missing: {stage_line}"
    );
}

#[test]
fn workflow_widget_renders_plain_text_like_quecto() {
    let mut state = make_state(Some(100), 3, 14);
    state.workflow_auto_continue = true;
    state.workflow_completion_nudge = false;
    let lines = render_widget(&state, 100);
    assert_eq!(lines.len(), 2);
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
    let hints = &lines[1];
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
fn workflow_widget_hidden_when_nothing_started() {
    let state = make_state(None, 0, 14);
    assert!(render_widget(&state, 100).is_empty());
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
fn workflow_widget_complete_shows_done() {
    let state = make_state(Some(100), 14, 14);
    let lines = render_widget(&state, 100);
    assert!(lines[0].contains("✓ Workflow complete!"));
}

#[test]
fn all_done_shows_done() {
    let state = make_state(Some(100), 14, 14);
    let line = &render(&state, 80)[1];
    assert!(
        line.contains("DONE") || line.contains("Complete"),
        "should show done state: {}",
        line
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
fn active_mode_renders_template_name() {
    let mut state = make_state(Some(100), 3, 14);
    state.template_name = Some("my-template".into());
    let line = &render(&state, 80)[1];
    assert!(line.contains("my-template"));
}

#[test]
fn selector_mode_renders_select_template() {
    let mut state = make_state(None, 0, 0);
    state.mode = Some("selecting_template".into());
    state.template_count = 4;
    let line = &render(&state, 80)[1];
    assert!(line.contains("SELECT TEMPLATE"));
    assert!(line.contains("4 available"));
}

#[test]
fn selector_mode_visible_even_without_issue() {
    let mut state = make_state(None, 0, 0);
    state.mode = Some("selecting_template".into());
    state.template_count = 2;
    assert!(state.is_visible());
}

#[test]
fn selector_mode_has_blank_line_above_and_below() {
    let mut state = make_state(None, 0, 0);
    state.mode = Some("selecting_template".into());
    state.template_count = 2;
    let lines = render(&state, 80);
    assert_eq!(lines.len(), 4);
    assert!(lines[0].is_empty());
    assert!(lines[3].is_empty());
}

#[test]
fn active_bar_has_blank_line_above_and_below() {
    let state = make_state(Some(100), 3, 14);
    let lines = render(&state, 80);
    assert_eq!(lines.len(), 4);
    assert!(lines[0].is_empty());
    assert!(lines[3].is_empty());
}
