//! Sci-fi styled workflow progress bar for the TUI header (#563).
//!
//! Renders a single-line progress indicator showing issue number, title,
//! progress bar, and current phase. Hidden when no workflow issue is active.

use crate::theme;

/// Workflow step info received from a `workflow_state` event.
#[derive(Debug, Clone)]
pub struct WorkflowStepInfo {
    pub id: u32,
    pub phase: String,
    pub done: bool,
}

/// Workflow state for the header bar.
#[derive(Debug, Clone, Default)]
pub struct WorkflowBarState {
    pub steps: Vec<WorkflowStepInfo>,
    pub done: u32,
    pub total: u32,
    pub issue_number: Option<u32>,
    pub issue_title: Option<String>,
    /// V2: workflow mode (selecting_template, active, complete).
    pub mode: Option<String>,
    /// V2: active template display name.
    pub template_name: Option<String>,
    /// V2: number of available templates (for selector mode display).
    pub template_count: u32,
}

impl WorkflowBarState {
    /// Whether the bar should be visible.
    pub fn is_visible(&self) -> bool {
        // V2: visible in selector mode even without an issue.
        if self.mode.as_deref() == Some("selecting_template") {
            return true;
        }
        self.issue_number.is_some() && self.total > 0
    }

    /// Find the current phase (phase of the first unchecked step).
    pub fn current_phase(&self) -> Option<&str> {
        self.steps
            .iter()
            .find(|s| !s.done)
            .map(|s| s.phase.as_str())
    }

    /// Find the current step label for display.
    pub fn current_step_id(&self) -> Option<u32> {
        self.steps.iter().find(|s| !s.done).map(|s| s.id)
    }
}

/// Render the sci-fi workflow header bar.
///
/// Returns an empty vec if no workflow is active.
/// Returns a single styled line if active.
pub fn render(state: &WorkflowBarState, width: usize) -> Vec<String> {
    if !state.is_visible() {
        return vec![];
    }

    let reset = "\x1b[0m";

    // V2: Selector mode — no progress, just template selection prompt.
    if state.mode.as_deref() == Some("selecting_template") {
        let bg = "\x1b[48;2;20;15;40m"; // subtle purple tint
        let issue_part = state
            .issue_number
            .map(|n| format!(" #{} \u{2500}\u{2500}\u{2500}", n))
            .unwrap_or_default();
        let content = format!(
            " \u{2590} WF{} \u{27E8}{}\u{27E9} SELECT TEMPLATE ({} available)",
            issue_part,
            theme::bold("SELECT"),
            state.template_count,
        );
        let vis_width = crate::utils::visible_width(&content);
        let padding = if vis_width < width {
            " ".repeat(width - vis_width)
        } else {
            String::new()
        };
        return vec![
            String::new(),
            format!("{bg}{content}{padding}{reset}"),
            String::new(),
        ];
    }

    let issue_num = state.issue_number.unwrap_or(0);
    let issue_title: String = state
        .issue_title
        .as_deref()
        .unwrap_or("")
        .chars()
        .filter(|c| !c.is_control())
        .take(30)
        .collect();

    // Progress bar
    let bar_width: usize = 12;
    let filled = if state.total > 0 {
        ((state.done as usize) * bar_width) / (state.total as usize)
    } else {
        0
    };
    let empty = bar_width.saturating_sub(filled);
    let progress_bar = format!("{}{}", "▓".repeat(filled), "░".repeat(empty));

    // Phase tag
    let phase = state.current_phase().unwrap_or("DONE");
    let phase_display = phase_name(phase);

    // Template name (V2) or step info fallback
    let template_part = state
        .template_name
        .as_deref()
        .map(|name| format!(" [{}]", name))
        .unwrap_or_default();

    // Step info
    let step_info = if let Some(step_id) = state.current_step_id() {
        format!("Step {}", step_id)
    } else {
        "Complete".to_string()
    };

    // Build the line with phase-aware background
    let bg = phase_bg(phase);

    let content = format!(
        " \u{2590} WF #{} \u{2500}\u{2500}\u{2500} {}{} \u{2500}\u{2500}\u{2500} {} {:02}/{:02} \u{2500}\u{2500}\u{2500} \u{27E8}{}\u{27E9} {}",
        issue_num,
        issue_title,
        template_part,
        theme::dim(&progress_bar),
        state.done,
        state.total,
        theme::bold(phase_display),
        theme::dim(&step_info),
    );

    // Pad to width and apply background
    let vis_width = crate::utils::visible_width(&content);
    let padding = if vis_width < width {
        " ".repeat(width - vis_width)
    } else {
        String::new()
    };

    vec![
        String::new(),
        format!("{}{}{}{}", bg, content, padding, reset),
        String::new(),
    ]
}

/// Phase display name.
fn phase_name(phase: &str) -> &str {
    match phase {
        "red" => "RED",
        "green" => "GREEN",
        "refactor" => "REFACTOR",
        "ci_cd" => "CI/CD",
        "review" => "REVIEW",
        _ => "DONE",
    }
}

/// True-colour background tint per phase (Alacritty-safe).
fn phase_bg(phase: &str) -> &'static str {
    match phase {
        "red" => "\x1b[48;2;40;10;10m",
        "green" => "\x1b[48;2;10;40;10m",
        "ci_cd" => "\x1b[48;2;10;10;40m",
        "review" => "\x1b[48;2;40;30;10m",
        _ => "\x1b[48;2;15;15;15m", // Done / unknown — subtle dark
    }
}

/// Parse a `workflow_state` JSON event into `WorkflowBarState`.
pub fn parse_workflow_event(data: &serde_json::Value) -> WorkflowBarState {
    let steps: Vec<WorkflowStepInfo> = data
        .get("steps")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| {
                    // V2: field is "index"; V1 compat: "id"
                    let id = s.get("index").or_else(|| s.get("id"))?.as_u64()? as u32;
                    Some(WorkflowStepInfo {
                        id,
                        phase: s.get("phase")?.as_str()?.to_string(),
                        done: s.get("done")?.as_bool()?,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let done = data
        .get("progress")
        .and_then(|p| p.get("done"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;
    let total = data
        .get("progress")
        .and_then(|p| p.get("total"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    // Handle both camelCase (workflow_state event) and snake_case (get_state response).
    let issue_number = data
        .get("activeIssue")
        .or_else(|| data.get("active_issue"))
        .and_then(|i| i.get("number").or_else(|| i.as_array().and_then(|a| a.first())))
        .and_then(|v| v.as_u64())
        .map(|n| n as u32);
    let issue_title = data
        .get("activeIssue")
        .or_else(|| data.get("active_issue"))
        .and_then(|i| i.get("title").or_else(|| i.as_array().and_then(|a| a.get(1))))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let mode = data
        .get("mode")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let template_name = data
        .get("activeTemplate")
        .or_else(|| data.get("active_template"))
        .and_then(|t| t.get("label"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let template_count = data
        .get("availableTemplates")
        .or_else(|| data.get("available_templates"))
        .and_then(|v| v.as_array())
        .map(|a| a.len() as u32)
        .unwrap_or(0);

    WorkflowBarState {
        steps,
        done,
        total,
        issue_number,
        issue_title,
        mode,
        template_name,
        template_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state(issue: Option<u32>, done: u32, total: u32) -> WorkflowBarState {
        let steps: Vec<WorkflowStepInfo> = (1..=total)
            .map(|i| WorkflowStepInfo {
                id: i,
                phase: if i <= 3 {
                    "red".into()
                } else if i <= 4 {
                    "green".into()
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
        assert_eq!(lines.len(), 3, "blank + content + blank");
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
    fn current_phase_returns_first_unchecked() {
        let state = make_state(Some(1), 3, 14);
        assert_eq!(state.current_phase(), Some("green"));
    }

    #[test]
    fn current_phase_none_when_all_done() {
        let state = make_state(Some(1), 14, 14);
        assert!(state.current_phase().is_none());
    }

    // ── V2 tests: mode, template, selector ──────────────────────────

    #[test]
    fn parse_v2_steps_with_index_field() {
        let event = serde_json::json!({
            "steps": [
                {"index": 1, "label": "Scope", "phase": "red", "done": true},
                {"index": 2, "label": "Change", "phase": "green", "done": false},
            ],
            "progress": {"done": 1, "total": 2, "percent": 50},
        });
        let state = parse_workflow_event(&event);
        assert_eq!(state.steps.len(), 2);
        assert_eq!(state.steps[0].id, 1);
        assert_eq!(state.steps[1].id, 2);
    }

    #[test]
    fn parse_get_state_snake_case_fields() {
        let event = serde_json::json!({
            "mode": "active",
            "active_template": {"id": "chore", "label": "Chore"},
            "available_templates": [
                {"id": "feature", "label": "Feature"},
                {"id": "chore", "label": "Chore"},
            ],
            "active_issue": [99, "Fix auth"],
            "steps": [
                {"index": 1, "phase": "red", "done": true},
            ],
            "progress": {"done": 1, "total": 1, "percent": 100},
        });
        let state = parse_workflow_event(&event);
        assert_eq!(state.mode.as_deref(), Some("active"));
        assert_eq!(state.template_name.as_deref(), Some("Chore"));
        assert_eq!(state.template_count, 2);
        assert_eq!(state.issue_number, Some(99));
        assert_eq!(state.issue_title.as_deref(), Some("Fix auth"));
    }

    #[test]
    fn parse_v2_event_captures_mode() {
        let event = serde_json::json!({
            "type": "workflow_state",
            "mode": "selecting_template",
            "steps": [],
            "progress": {"done": 0, "total": 0, "percent": 0},
            "availableTemplates": [
                {"id": "feature", "label": "Feature"},
                {"id": "fix", "label": "Fix"},
            ],
        });
        let state = parse_workflow_event(&event);
        assert_eq!(state.mode.as_deref(), Some("selecting_template"));
        assert_eq!(state.template_count, 2);
    }

    #[test]
    fn parse_v2_event_captures_template_name() {
        let event = serde_json::json!({
            "type": "workflow_state",
            "mode": "active",
            "activeTemplate": {"id": "fix", "label": "Fix"},
            "steps": [
                {"id": 1, "label": "Reproduce", "phase": "red", "done": true},
                {"id": 2, "label": "Test", "phase": "red", "done": false},
            ],
            "progress": {"done": 1, "total": 2, "percent": 50},
            "activeIssue": {"number": 42, "title": "auth bug"},
        });
        let state = parse_workflow_event(&event);
        assert_eq!(state.mode.as_deref(), Some("active"));
        assert_eq!(state.template_name.as_deref(), Some("Fix"));
    }

    #[test]
    fn selector_mode_visible_even_without_issue() {
        let mut state = WorkflowBarState::default();
        state.mode = Some("selecting_template".into());
        state.template_count = 4;
        assert!(state.is_visible(), "selector mode should be visible");
    }

    #[test]
    fn selector_mode_renders_select_template() {
        let mut state = WorkflowBarState::default();
        state.mode = Some("selecting_template".into());
        state.template_count = 4;
        let lines = render(&state, 80);
        assert!(!lines.is_empty(), "selector mode should render");
        let line = &lines[1];
        assert!(
            line.contains("SELECT") || line.contains("TEMPLATE"),
            "selector mode should mention template selection: {line}"
        );
    }

    #[test]
    fn active_mode_renders_template_name() {
        let mut state = make_state(Some(42), 2, 6);
        state.mode = Some("active".into());
        state.template_name = Some("Fix".into());
        let lines = render(&state, 100);
        assert!(!lines.is_empty());
        // Content is in the middle line (between blank spacers).
        let content_line = lines.iter().find(|l| l.contains("Fix")).unwrap();
        assert!(
            content_line.contains("Fix"),
            "active mode should show template name: {content_line}"
        );
    }

    // ── Vertical spacing (#600) ─────────────────────────────────────

    #[test]
    fn active_bar_has_blank_line_above_and_below() {
        let state = make_state(Some(42), 3, 14);
        let lines = render(&state, 80);
        assert_eq!(lines.len(), 3, "expected blank + content + blank, got {lines:?}");
        assert!(lines[0].trim().is_empty(), "first line should be blank");
        assert!(!lines[1].trim().is_empty(), "middle line should be content");
        assert!(lines[2].trim().is_empty(), "last line should be blank");
    }

    #[test]
    fn selector_mode_has_blank_line_above_and_below() {
        let mut state = WorkflowBarState::default();
        state.mode = Some("selecting_template".into());
        state.template_count = 4;
        let lines = render(&state, 80);
        assert_eq!(lines.len(), 3, "expected blank + content + blank, got {lines:?}");
        assert!(lines[0].trim().is_empty(), "first line should be blank");
        assert!(lines[1].contains("SELECT"), "middle line should be content");
        assert!(lines[2].trim().is_empty(), "last line should be blank");
    }

    #[test]
    fn hidden_bar_returns_no_lines() {
        let state = make_state(None, 0, 14);
        let lines = render(&state, 80);
        assert!(lines.is_empty(), "hidden bar should return no lines");
    }
}
