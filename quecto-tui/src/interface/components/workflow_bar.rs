//! Workflow progress UI for the TUI.
//!
//! Matches the Pi workflow extension: the widget is a single plain text line
//! above the editor with no background, and the checklist panel is a read-only
//! mirror of the Pi WorkflowChecklist.

use crate::interface::theme;

/// Workflow step info received from a `workflow_state` event.
#[derive(Debug, Clone)]
pub struct WorkflowStepInfo {
    pub id: u32,
    pub label: String,
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
    /// TUI-local: whether workflow auto-continue is enabled.
    pub workflow_auto_continue: bool,
    /// TUI-local: whether completion nudge is enabled.
    pub workflow_completion_nudge: bool,
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

    /// Find the current step title for display.
    pub fn current_step_label(&self) -> Option<&str> {
        self.steps
            .iter()
            .find(|s| !s.done)
            .map(|s| s.label.as_str())
    }
}

/// Render the sci-fi workflow header bar.
///
/// Returns an empty vec if no workflow is active.
/// Returns four lines if active: blank spacer, styled content, stage/hotkey tips, blank spacer.
///
/// Kept for backwards compatibility but no longer used in the main app render.
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
        let vis_width = crate::interface::utils::visible_width(&content);
        let padding = if vis_width < width {
            " ".repeat(width - vis_width)
        } else {
            String::new()
        };
        let tips = format!(" {}", theme::dim("Ctrl+Shift+A auto · Ctrl+Shift+N nudge"));
        let tips = pad_or_truncate_with_bg(&tips, width, bg, reset);
        return vec![
            String::new(),
            format!("{bg}{content}{padding}{reset}"),
            tips,
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
        let label = state
            .current_step_label()
            .map(short_step_label)
            .unwrap_or_default();
        if label.is_empty() {
            format!("Step {}", step_id)
        } else {
            format!("Step {}: {}", step_id, label)
        }
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
    let vis_width = crate::interface::utils::visible_width(&content);
    let padding = if vis_width < width {
        " ".repeat(width - vis_width)
    } else {
        String::new()
    };

    let stage_line = render_stage_status_line(state, width, bg, reset);

    vec![
        String::new(),
        format!("{}{}{}{}", bg, content, padding, reset),
        stage_line,
        String::new(),
    ]
}

/// Render the Pi-style workflow widget above the editor.
///
/// Matches the Pi extension's `updateWidget` implementation:
/// - plain text line, no background
/// - hidden when `done == 0 && !activeIssue`
/// - content: `Workflow #ISSUE TITLE [bar] done/total (pct%) → Step N: label [PHASE]`
///   or `✓ Workflow complete!` when all steps are done.
pub fn render_widget(state: &WorkflowBarState, width: usize) -> Vec<String> {
    if width == 0 || !is_widget_visible(state) {
        return vec![];
    }

    let done = state.done;
    let total = state.total.max(state.steps.len() as u32).max(1);
    let pct = ((done as f32 / total as f32) * 100.0).round() as u32;
    let filled = ((done as usize) * 15) / (total as usize);
    let bar = format!(
        "{}{}",
        theme::success(&"█".repeat(filled)),
        theme::dim(&"░".repeat(15 - filled))
    );

    let issue_part = match (state.issue_number, state.issue_title.as_deref()) {
        (Some(number), Some(title)) => format!(
            " {}{} ",
            theme::accent(&theme::bold(&format!("#{number}"))),
            theme::dim(&ellipsize_clean(title, 40))
        ),
        (Some(number), None) => format!(" {}", theme::accent(&theme::bold(&format!("#{number}")))),
        _ => " ".to_string(),
    };

    let current_info = state
        .current_step_id()
        .map(|id| {
            let label = state.current_step_label().unwrap_or("");
            format!(
                "→ Step {id}: {} [{}]",
                ellipsize_clean(label, 56),
                phase_label_for_widget(state.current_phase().unwrap_or("done"))
            )
        })
        .unwrap_or_else(|| "✓ Workflow complete!".to_string());

    let line = format!(
        "{}{}{}{}{}",
        theme::accent(&theme::bold("Workflow")),
        issue_part,
        bar,
        theme::muted(&format!(" {done}/{total} ({pct}%) ")),
        theme::dim(&current_info)
    );

    let auto = if state.workflow_auto_continue {
        "on"
    } else {
        "off"
    };
    let nudge = if state.workflow_completion_nudge {
        "on"
    } else {
        "off"
    };
    let hints = format!(
        "  {}",
        theme::dim(&format!(
            "Ctrl+Shift+A auto:{auto} · Ctrl+Shift+N nudge:{nudge}"
        ))
    );

    vec![truncate_line(&line, width), truncate_line(&hints, width)]
}

fn is_widget_visible(state: &WorkflowBarState) -> bool {
    // Match Pi extension: hide when nothing is started and no active issue.
    state.done > 0 || state.issue_number.is_some()
}

fn truncate_line(text: &str, width: usize) -> String {
    crate::interface::utils::truncate_to_width(text, width, Some("…"))
}

fn sanitize_text(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

fn ellipsize_clean(text: &str, max_chars: usize) -> String {
    let clean = sanitize_text(text);
    let mut out: String = clean.chars().take(max_chars).collect();
    if clean.chars().count() > max_chars {
        out.push('…');
    }
    out
}

fn phase_label_for_widget(phase: &str) -> &str {
    phase_name(phase)
}

fn render_stage_status_line(
    state: &WorkflowBarState,
    width: usize,
    bg: &str,
    reset: &str,
) -> String {
    let stages = ["red", "green", "refactor", "ci_cd", "review"];
    let current_phase = state.current_phase();
    let mut parts = Vec::new();
    for phase in stages {
        let phase_steps: Vec<&WorkflowStepInfo> = state
            .steps
            .iter()
            .filter(|step| step.phase == phase || (phase == "ci_cd" && step.phase == "ci"))
            .collect();
        let marker = if !phase_steps.is_empty() && phase_steps.iter().all(|step| step.done) {
            theme::success("✓")
        } else if current_phase == Some(phase) || (phase == "ci_cd" && current_phase == Some("ci"))
        {
            theme::accent("●")
        } else {
            theme::dim("○")
        };
        parts.push(format!("{} {}", marker, phase_name(phase)));
    }

    let auto = if state.workflow_auto_continue {
        "on"
    } else {
        "off"
    };
    let nudge = if state.workflow_completion_nudge {
        "on"
    } else {
        "off"
    };
    let tips = theme::dim(&format!(
        "Ctrl+Shift+A:auto {auto} · Ctrl+Shift+N:nudge {nudge}"
    ));
    let line = format!("  {}   {}", parts.join("  "), tips);
    pad_or_truncate_with_bg(&line, width, bg, reset)
}

fn pad_or_truncate_with_bg(text: &str, width: usize, bg: &str, reset: &str) -> String {
    let text = crate::interface::utils::truncate_to_width(text, width, Some("…"));
    let vis_width = crate::interface::utils::visible_width(&text);
    let padding = if vis_width < width {
        " ".repeat(width - vis_width)
    } else {
        String::new()
    };
    format!("{bg}{text}{padding}{reset}")
}

fn short_step_label(label: &str) -> String {
    label.chars().filter(|c| !c.is_control()).take(28).collect()
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
                        label: s
                            .get("label")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
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
        .map(|n| n as u32)
        .unwrap_or_else(|| steps.iter().filter(|s| s.done).count() as u32);
    let total = data
        .get("progress")
        .and_then(|p| p.get("total"))
        .and_then(|v| v.as_u64())
        .map(|n| n as u32)
        .unwrap_or(steps.len() as u32);

    // Handle both camelCase (workflow_state event) and snake_case (get_state response).
    let issue_number = data
        .get("activeIssue")
        .or_else(|| data.get("active_issue"))
        .and_then(|i| {
            i.get("number")
                .or_else(|| i.as_array().and_then(|a| a.first()))
        })
        .and_then(|v| v.as_u64())
        .map(|n| n as u32);
    let issue_title = data
        .get("activeIssue")
        .or_else(|| data.get("active_issue"))
        .and_then(|i| {
            i.get("title")
                .or_else(|| i.as_array().and_then(|a| a.get(1)))
        })
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
        workflow_auto_continue: false,
        workflow_completion_nudge: false,
    }
}

#[cfg(test)]
mod tests {
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
    fn workflow_widget_renders_plain_text_like_pi() {
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
}
