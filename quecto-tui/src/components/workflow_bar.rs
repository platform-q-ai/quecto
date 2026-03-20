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
}

impl WorkflowBarState {
    /// Whether the bar should be visible.
    pub fn is_visible(&self) -> bool {
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

    let issue_num = state.issue_number.unwrap_or(0);
    let issue_title = state
        .issue_title
        .as_deref()
        .unwrap_or("")
        .chars()
        .take(30)
        .collect::<String>();

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

    // Step info
    let step_info = if let Some(step_id) = state.current_step_id() {
        format!("Step {}", step_id)
    } else {
        "Complete".to_string()
    };

    // Build the line with phase-aware background
    let bg = phase_bg(phase);
    let reset = "\x1b[0m";

    let content = format!(
        " \u{2590} WF #{} \u{2500}\u{2500}\u{2500} {} \u{2500}\u{2500}\u{2500} {} {:02}/{:02} \u{2500}\u{2500}\u{2500} \u{27E8}{}\u{27E9} {}",
        issue_num,
        issue_title,
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

    vec![format!("{}{}{}{}", bg, content, padding, reset)]
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
                    Some(WorkflowStepInfo {
                        id: s.get("id")?.as_u64()? as u32,
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

    let issue_number = data
        .get("activeIssue")
        .and_then(|i| i.get("number"))
        .and_then(|v| v.as_u64())
        .map(|n| n as u32);
    let issue_title = data
        .get("activeIssue")
        .and_then(|i| i.get("title"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    WorkflowBarState {
        steps,
        done,
        total,
        issue_number,
        issue_title,
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
        assert_eq!(lines.len(), 1);
    }

    #[test]
    fn contains_issue_number() {
        let state = make_state(Some(559), 4, 14);
        let line = &render(&state, 80)[0];
        assert!(
            line.contains("559"),
            "should contain issue number: {}",
            line
        );
    }

    #[test]
    fn contains_progress_fraction() {
        let state = make_state(Some(100), 7, 14);
        let line = &render(&state, 80)[0];
        assert!(line.contains("07/14"), "should contain progress: {}", line);
    }

    #[test]
    fn contains_phase_name() {
        let state = make_state(Some(100), 3, 14);
        let line = &render(&state, 80)[0];
        assert!(line.contains("GREEN"), "step 4 is green phase: {}", line);
    }

    #[test]
    fn contains_progress_bar_chars() {
        let state = make_state(Some(100), 7, 14);
        let line = &render(&state, 80)[0];
        assert!(line.contains('▓'), "should contain filled block");
        assert!(line.contains('░'), "should contain empty block");
    }

    #[test]
    fn contains_box_drawing() {
        let state = make_state(Some(100), 4, 14);
        let line = &render(&state, 80)[0];
        assert!(line.contains('─'), "should contain box drawing dash");
        assert!(line.contains('▐'), "should contain right half block sigil");
    }

    #[test]
    fn contains_angle_brackets() {
        let state = make_state(Some(100), 4, 14);
        let line = &render(&state, 80)[0];
        assert!(line.contains('⟨'), "should contain left angle bracket");
        assert!(line.contains('⟩'), "should contain right angle bracket");
    }

    #[test]
    fn contains_true_colour_bg() {
        let state = make_state(Some(100), 0, 14);
        let line = &render(&state, 80)[0];
        // RED phase bg
        assert!(
            line.contains("\x1b[48;2;"),
            "should contain true-colour bg escape"
        );
    }

    #[test]
    fn all_done_shows_done() {
        let state = make_state(Some(100), 14, 14);
        let line = &render(&state, 80)[0];
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
}
