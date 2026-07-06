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
        // V2: selector mode is visible only as a GENUINE selector affordance —
        // i.e. there are templates to choose, a template already selected, real
        // steps, or an active issue (#912). A DORMANT enabled-but-unselected
        // workflow (the master's connect-time `selecting_template`/`0/0` with no
        // steps, no templates and no active issue) must NOT render a spurious
        // `auto:on · starting…` bar.
        if self.mode.as_deref() == Some("selecting_template") {
            // A dormant selector — templates available but nothing selected —
            // must NOT render a spurious "0/1 auto:on starting…" bar on a fresh
            // `--workflow` boot (#912). Having templates to choose is not enough;
            // show the bar only once a workflow is genuinely engaged (real steps,
            // an active template, or an active issue). Template selection moves
            // the engine to `active` mode, where the bar then appears.
            return !self.steps.is_empty()
                || self.template_name.is_some()
                || self.issue_number.is_some();
        }
        // Show on selection (#901): a just-selected workflow with real steps
        // (e.g. `0/18`) renders immediately, not only once a step completes or
        // an issue is set.
        //
        // #903: a bare `total > 0` is NOT enough. The master's connect-time
        // snapshot can carry a stale `progress.total` from a persisted template
        // with an EMPTY `steps` array, no active issue and no active template —
        // an inactive state that previously read as a spurious "complete" bar.
        // Require a genuine active workflow: real steps, an active template, or
        // an active issue.
        !self.steps.is_empty() || self.template_name.is_some() || self.issue_number.is_some()
    }

    /// Whether the bar carries NO meaningful workflow content. Used by the
    /// per-agent routing stickiness guard (#901) to tell a transient/empty
    /// `workflow_state` event apart from a real one.
    pub fn is_empty(&self) -> bool {
        self.has_no_progress() && self.issue_number.is_none() && self.template_name.is_none()
    }

    /// Whether the event carries NO renderable PROGRESS: no steps, `0/0`, and no
    /// templates to select. Unlike [`is_empty`] this ignores `active_issue` and
    /// `mode`, so a transient `0/0`-with-issue `workflow_state` (#915) still
    /// counts as carrying no progress and must not regress an advanced bar down
    /// to `starting…`.
    pub fn has_no_progress(&self) -> bool {
        self.steps.is_empty() && self.done == 0 && self.total == 0 && self.template_count == 0
    }

    /// Seed a bar from a sub-agent registry snapshot (`SubagentWorkflow`) so
    /// selecting a child shows its main-pane bar IMMEDIATELY, matching the
    /// left-panel cells, without waiting for a routed `get_state`/live
    /// `workflow_state` (#913). The snapshot carries only `mode` + `done/total`
    /// (no per-step detail), so synthesize placeholder steps from the counts; the
    /// renderer then shows `Step n/total` instead of a misleading `starting…`.
    ///
    /// LIMITATION (#913 AC#3): the registry `SubagentWorkflow` snapshot carries no
    /// automation block (the kernel's sub-agent `WorkflowSnapshot` /
    /// `snapshot_to_event` do not propagate `autoContinue`/`completionNudge`), so a
    /// snapshot-seeded bar renders `auto:off` until the first routed `workflow_state`
    /// /`get_state` event with an `automation` block arrives (handled by
    /// `parse_workflow_event`). Threading the flag end-to-end through the kernel
    /// snapshot is intentionally out of scope for this display-cluster branch.
    pub fn from_subagent_snapshot(mode: &str, done: u32, total: u32) -> Self {
        // `total` comes from an untrusted sub-agent snapshot and the bar only
        // draws a fixed-width gauge from done/total — so cap the synthesized
        // placeholder steps to avoid a multi-GB allocation (one struct per step)
        // when a malicious/buggy child reports a huge `steps_total`, which would
        // OOM the operator's TUI merely on selecting that child (#919 security
        // review). The real `done`/`total` are kept below for the text readout.
        const MAX_SNAPSHOT_STEPS: u32 = 512;
        let synth = total.min(MAX_SNAPSHOT_STEPS);
        let steps: Vec<WorkflowStepInfo> = (0..synth)
            .map(|i| WorkflowStepInfo {
                id: i + 1,
                label: String::new(),
                phase: "build".to_string(),
                done: i < done,
            })
            .collect();
        WorkflowBarState {
            steps,
            done,
            total,
            mode: Some(mode.to_string()),
            ..Default::default()
        }
    }

    /// Whether the event explicitly signals a workflow END, so an
    /// otherwise-empty bar is allowed to CLEAR a currently-visible one (#901).
    ///
    /// The kernel only ever emits three `workflow_state.mode` values
    /// (`src/domain/workflow.rs`, `WorkflowMode::wire_str`):
    /// `"selecting_template"`, `"active"`, `"complete"`. Of these only
    /// `"complete"` is terminal; the other two are transient/intermediate and
    /// must NOT clear a visible workflow. (The forwarded descendant path —
    /// `canonical_workflow_forward` — drops steps/templates, so a forwarded
    /// `complete` whose progress has reset to `0/0` would otherwise look empty
    /// and stick; this lets it clear.)
    ///
    /// NOTE: a genuine reset of an UNBOUND engine re-enters the selector as
    /// `selecting_template`/`0/0`, which on the forwarded path is byte-identical
    /// to a `#899` transient and is therefore intentionally NOT treated as a
    /// clear — bound sub-agents (the common case) never return to the selector,
    /// so this preserves anti-flicker (AC#2/#4) at the cost of leaving an
    /// unbound reset sticky until the agent exits.
    pub fn signals_end_or_reset(&self) -> bool {
        matches!(self.mode.as_deref(), Some("complete"))
    }

    /// Whether the workflow is GENUINELY complete (#903): every step done
    /// (`done >= total > 0`) or the kernel signalled the terminal `complete`
    /// mode. An empty-steps / not-started snapshot (`done < total`, or no steps)
    /// is NOT complete and must never render "✓ Workflow complete!".
    pub fn is_complete(&self) -> bool {
        self.mode.as_deref() == Some("complete") || (self.total > 0 && self.done >= self.total)
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

/// Render a filled/empty progress bar `cells` wide from `done` of `total`.
///
/// Single source of truth for the progress-bar glyph and colour style, reused by
/// the main workflow bar and by the sub-agent inspector phase header (#795).
pub fn progress_bar(done: u32, total: u32, cells: usize) -> String {
    let total = total.max(1) as usize;
    let filled = ((done as usize) * cells / total).min(cells);
    format!(
        "{}{}",
        theme::success(&"█".repeat(filled)),
        theme::dim(&"░".repeat(cells - filled))
    )
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
    let bar = progress_bar(done, total, 15);

    let issue_part = match (state.issue_number, state.issue_title.as_deref()) {
        (Some(number), Some(title)) => format!(
            " {}{} ",
            theme::accent(&theme::bold(&format!("#{number}"))),
            theme::dim(&ellipsize_clean(title, 40))
        ),
        (Some(number), None) => format!(" {}", theme::accent(&theme::bold(&format!("#{number}")))),
        _ => " ".to_string(),
    };

    // #903: "✓ Workflow complete!" is reserved for a GENUINELY complete
    // workflow. When `current_step_id()` is `None` because steps are empty /
    // not-started (rather than all-done), render a neutral "starting…" marker
    // instead of falsely claiming completion.
    let current_info = match state.current_step_id() {
        Some(id) => {
            let label = state.current_step_label().unwrap_or("");
            format!(
                "→ Step {id}: {} [{}]",
                ellipsize_clean(label, 56),
                phase_display(state.current_phase().unwrap_or("done"))
            )
        }
        None if state.is_complete() => "✓ Workflow complete!".to_string(),
        None => "starting…".to_string(),
    };

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

/// Render the workflow as a SINGLE content line for the sub-agent-first main
/// pane (#820): `progress-bar  PHASE n/total` (e.g. `███████░░░  GREEN 3/4`).
/// Drops the phase-pills and hints lines of `render_widget`. Returns `None`
/// when the workflow is not visible (no issue / nothing started).
pub fn render_compact_line(state: &WorkflowBarState) -> Option<String> {
    if !is_widget_visible(state) {
        return None;
    }
    let total = state.total.max(state.steps.len() as u32).max(1);
    let bar = progress_bar(state.done, total, 10);

    // Concise current-step context (#882): `Step n/total PHASE · label · #issue`,
    // or a completion marker once every step is done. Long labels/issue titles
    // are ellipsized so the single boxed line never wraps.
    let context = match state.current_step_id() {
        Some(id) => {
            let phase = phase_display(state.current_phase().unwrap_or("done"));
            let label = state.current_step_label().unwrap_or("");
            let mut parts = format!(
                "{} {}",
                theme::accent(&theme::bold(&format!("Step {id}/{total}"))),
                theme::muted(&phase),
            );
            if !label.is_empty() {
                parts.push_str(&theme::dim(&format!(" · {}", ellipsize_clean(label, 32))));
            }
            parts
        }
        // #903: only label complete when genuinely done, never for empty steps.
        None if state.is_complete() => theme::success("✓ Workflow complete!"),
        None => theme::dim("starting…"),
    };

    // #897 AC2: surface auto_continue in the always-visible main pane so its
    // overriding of "wait"-style instructions is never surprising. Place it
    // ahead of the ellipsizable context/issue-number so it survives clipping
    // under narrow widths (the caller clamps to the box inner width).
    let auto = if state.workflow_auto_continue {
        "on"
    } else {
        "off"
    };
    // Surface the raw `done/total` progress count next to the bar so the
    // main-pane line carries the watermark explicitly (not just the current-step
    // ordinal) — this is the canonical progress readout used by the display
    // regression guard (#915).
    let mut line = format!(
        "{bar}  {} {}",
        theme::muted(&format!("{}/{}", state.done, total)),
        theme::dim(&format!("auto:{auto}"))
    );
    line.push_str(&theme::dim(" · "));
    line.push_str(&context);
    if let Some(number) = state.issue_number {
        line.push_str(&theme::dim(" · "));
        line.push_str(&theme::accent(&theme::bold(&format!("#{number}"))));
    }
    Some(line)
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
///
/// This unifies the former `phase_name`/`phase_label_for_widget` (step header)
/// and `phase_display` (phase pills) helpers, which previously diverged only on
/// the unknown-phase fallback — the header rendered a misleading `[DONE]` while
/// the pills upper-cased the raw key. The pill behaviour is kept for both, since
/// upper-casing the actual key is more informative for arbitrary V2 templates
/// than labelling an unrecognised phase "DONE". The common `done` path is
/// unchanged (`"done"` → `"DONE"` either way).
fn phase_display(phase: &str) -> String {
    match phase {
        "setup" => "SETUP".to_string(),
        "red" => "RED".to_string(),
        "green" => "GREEN".to_string(),
        "refactor" => "REFACTOR".to_string(),
        "ci_cd" => "CI/CD".to_string(),
        "review" => "REVIEW".to_string(),
        // Unknown phases come from wire data (forwarded sub-agent events) and
        // must be sanitized to prevent terminal control-sequence injection into
        // the always-visible main pane.
        other => crate::interface::ansi::sanitize_control(&other.to_uppercase()),
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
    // Show on selection (#901): visible once a workflow is started, has an
    // active issue, OR is a just-selected workflow with a known total (`0/N`).
    // Selector mode shows even before a total is known.
    state.is_visible() || state.done > 0
}

fn truncate_line(text: &str, width: usize) -> String {
    crate::interface::utils::truncate_to_width(text, width, Some("…"))
}

fn ellipsize_clean(text: &str, max_chars: usize) -> String {
    crate::interface::utils::sanitize_truncate_chars_with_ellipsis(text, max_chars, "…")
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

    // #913: source the real automation flags from the snapshot's `automation`
    // block when present (the `get_state` `workflow` payload and the kernel's
    // workflow snapshot carry `automation.autoContinue`/`completionNudge`),
    // instead of hardcoding `auto:off`. Live `workflow_state` events that omit
    // the block still default to `false`, consistent with the prior behaviour.
    let automation = data.get("automation");
    let workflow_auto_continue = automation
        .and_then(|a| a.get("autoContinue").or_else(|| a.get("auto_continue")))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let workflow_completion_nudge = automation
        .and_then(|a| {
            a.get("completionNudge")
                .or_else(|| a.get("completion_nudge"))
        })
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    WorkflowBarState {
        steps,
        done,
        total,
        issue_number,
        issue_title,
        mode,
        template_name,
        template_count,
        workflow_auto_continue,
        workflow_completion_nudge,
    }
}

#[cfg(test)]
#[path = "workflow_bar_tests.rs"]
mod tests;
