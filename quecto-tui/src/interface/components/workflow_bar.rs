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

    // `▸ Workflow` panel header mirrors the subagent bar's `▸ Subagents` so the
    // two widgets read as sibling panels with a shared left gutter.
    let line = format!(
        "  {} {}{}{}{}{}",
        theme::dim("▸"),
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
        "    {}",
        theme::dim(&format!(
            "Ctrl+Shift+A auto:{auto} · Ctrl+Shift+N nudge:{nudge}"
        ))
    );

    let mut out = vec![truncate_line(&line, width)];
    // Phase-pill overview, derived from the actual steps so it generalises to
    // arbitrary V2 templates rather than the hardcoded TDD phase set.
    if let Some(pills) = phase_pill_line(state) {
        out.push(truncate_line(&pills, width));
    }
    out.push(truncate_line(&hints, width));
    out
}

/// Normalise phase keys so synonyms collapse to one pill (`ci` → `ci_cd`).
fn normalize_phase(phase: &str) -> &str {
    match phase {
        "ci" => "ci_cd",
        other => other,
    }
}

/// Display label for a phase: known phases use their canonical name, unknown
/// (custom-template) phases fall back to their upper-cased key.
fn phase_display(phase: &str) -> String {
    match phase {
        "setup" => "SETUP".to_string(),
        "red" => "RED".to_string(),
        "green" => "GREEN".to_string(),
        "refactor" => "REFACTOR".to_string(),
        "ci_cd" => "CI/CD".to_string(),
        "review" => "REVIEW".to_string(),
        other => other.to_uppercase(),
    }
}

/// Build the phase-pill overview line: one marker per distinct phase, in the
/// order phases first appear in the step list. `✓` done, `●` current, `○` pending.
/// Returns `None` when there are no steps to summarise.
fn phase_pill_line(state: &WorkflowBarState) -> Option<String> {
    let mut phases: Vec<&str> = Vec::new();
    for step in &state.steps {
        let p = normalize_phase(&step.phase);
        if !phases.contains(&p) {
            phases.push(p);
        }
    }
    if phases.is_empty() {
        return None;
    }
    let current = state.current_phase().map(normalize_phase);
    let parts: Vec<String> = phases
        .iter()
        .map(|&p| {
            let all_done = state
                .steps
                .iter()
                .filter(|s| normalize_phase(&s.phase) == p)
                .all(|s| s.done);
            let marker = if all_done {
                theme::success("✓")
            } else if current == Some(p) {
                theme::accent("●")
            } else {
                theme::dim("○")
            };
            format!("{} {}", marker, phase_display(p))
        })
        .collect();
    // Nested under the `▸ Workflow` header (column 4), aligned with the hints row.
    Some(format!("    {}", parts.join("  ")))
}

fn is_widget_visible(state: &WorkflowBarState) -> bool {
    // Match Quecto workflow: hide when nothing is started and no active issue.
    state.done > 0 || state.issue_number.is_some()
}

fn truncate_line(text: &str, width: usize) -> String {
    crate::interface::utils::truncate_to_width(text, width, Some("…"))
}

fn sanitize_text(text: &str) -> String {
    crate::interface::ansi::sanitize_control(text)
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
