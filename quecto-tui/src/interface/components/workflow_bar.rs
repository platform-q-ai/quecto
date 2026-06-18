//! Workflow progress UI for the TUI.
//!
//! Matches the Quecto workflow extension: the widget is a single plain text line
//! above the editor with no background, and the checklist panel is a read-only
//! mirror of the Quecto WorkflowChecklist.

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
    /// Whether core workflow auto-continue is enabled.
    pub workflow_auto_continue: bool,
    /// Whether core workflow completion nudge is enabled.
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

/// Render the Quecto-style workflow widget above the editor.
///
/// Matches the Quecto workflow's `updateWidget` implementation:
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
    // Match Quecto workflow: hide when nothing is started and no active issue.
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
    let stages = ["setup", "red", "green", "refactor", "ci_cd", "review"];
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
        "setup" => "SETUP",
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
        "setup" => "\x1b[48;2;25;25;45m",
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
#[path = "workflow_bar_tests.rs"]
mod tests;
