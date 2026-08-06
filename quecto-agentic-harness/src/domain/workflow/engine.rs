use super::*;

impl WorkflowEngine {
    pub fn new(config: WorkflowConfig, guards_enabled: bool) -> Result<Self, WorkflowError> {
        let templates = if config.templates.is_empty() {
            default_templates()
        } else {
            config.templates
        };
        validate_templates(&templates)?;
        Ok(Self {
            templates,
            run: WorkflowRun::default(),
            auto_continue: config.auto_continue,
            completion_nudge: config.completion_nudge,
            guards_enabled,
            selector_prompt: config.selector_prompt,
            bound: false,
            selector_nudge: false,
        })
    }

    /// Bind the engine to its currently-selected template (a by-value
    /// `--workflow-spec` assignment). Once bound, the model cannot switch or
    /// re-select a different template. See [`WorkflowEngine`].
    pub fn set_bound(&mut self, bound: bool) {
        self.bound = bound;
    }

    /// Whether this engine is bound to a single assigned template.
    pub fn is_bound(&self) -> bool {
        self.bound
    }

    /// Arm (or disarm) the idle-boundary template-selector nudge (#1113).
    /// Only explicit workflow sessions (`--workflow`) arm this: a plain UDS
    /// session with the workflow tool merely available must never be nudged
    /// to pick a template.
    pub fn set_selector_nudge(&mut self, enabled: bool) {
        self.selector_nudge = enabled;
    }

    pub fn restore_run(&mut self, persisted: WorkflowRunPersisted) {
        let active_issue = persisted
            .active_issue
            .map(|(number, title)| (number, truncate_issue_title(title)));

        if let Some(ref template_id) = persisted.template_id
            && let Some(template_index) = self.templates.iter().position(|t| &t.id == template_id)
        {
            let template = &self.templates[template_index];
            let mut done = persisted.done;
            done.resize(template.steps.len(), false);
            let mut seen_gap = false;
            for flag in &mut done {
                if seen_gap && *flag {
                    *flag = false;
                }
                if !*flag {
                    seen_gap = true;
                }
            }
            self.run = WorkflowRun {
                template_id: Some(template_id.clone()),
                template_index: Some(template_index),
                done,
                active_issue,
            };
            return;
        }

        if active_issue.is_some() {
            self.run = WorkflowRun {
                active_issue,
                ..WorkflowRun::default()
            };
            return;
        }

        self.run = WorkflowRun::default();
    }

    pub fn persisted_run(&self) -> Option<WorkflowRunPersisted> {
        if self.run.template_id.is_none() && self.run.active_issue.is_none() {
            return None;
        }
        Some(WorkflowRunPersisted {
            template_id: self.run.template_id.clone(),
            done: self.run.done.clone(),
            active_issue: self.run.active_issue.clone(),
        })
    }

    pub fn guards_enabled(&self) -> bool {
        self.guards_enabled
    }

    pub fn auto_continue_enabled(&self) -> bool {
        self.auto_continue
    }

    pub fn completion_nudge_enabled(&self) -> bool {
        self.completion_nudge
    }

    pub fn set_automation(&mut self, auto_continue: bool, completion_nudge: bool) {
        self.auto_continue = auto_continue;
        self.completion_nudge = completion_nudge;
    }

    pub fn mode(&self) -> WorkflowMode {
        let Some(template) = self.active_template() else {
            return WorkflowMode::SelectingTemplate;
        };
        if self.run.done.iter().take(template.steps.len()).all(|d| *d) {
            WorkflowMode::Complete
        } else {
            WorkflowMode::Active
        }
    }

    pub fn list_templates(&self) -> Vec<WorkflowTemplateSummary> {
        self.templates.iter().map(summary_for_template).collect()
    }

    /// Activate the given template and reset step progress for a new live run.
    ///
    /// When `issue` is `Some`, it replaces the current active issue.
    /// When `issue` is `None`, any existing active issue is preserved; call
    /// [`Self::clear_issue`] first if you need to clear it before selecting a
    /// template.
    pub fn select_template(
        &mut self,
        template_id: &str,
        issue: Option<(u32, String)>,
    ) -> Result<(), WorkflowError> {
        // A bound engine is pinned to its assigned template; it may re-select
        // that same template (used by `reset`) but never switch to another.
        if self.bound {
            if let Some(current) = self.run.template_id.as_deref() {
                if current != template_id {
                    return Err(WorkflowError::UnknownTemplate(format!(
                        "agent is bound to workflow template '{}' and cannot select '{}'",
                        current, template_id
                    )));
                }
            }
        }
        let template_index = self
            .templates
            .iter()
            .position(|t| t.id == template_id)
            .ok_or_else(|| {
                WorkflowError::UnknownTemplate(format!("unknown template: {}", template_id))
            })?;
        let template = &self.templates[template_index];
        self.run.template_id = Some(template.id.clone());
        self.run.template_index = Some(template_index);
        self.run.done = vec![false; template.steps.len()];
        if let Some((number, title)) = issue {
            self.run.active_issue = Some((number, truncate_issue_title(title)));
        }
        Ok(())
    }

    pub fn check(&mut self, step: u32) -> Result<(), WorkflowError> {
        let template = self.require_active_template()?;
        let idx = validate_step_index(step, template.steps.len())?;
        for i in 0..idx {
            if !self.run.done[i] {
                return Err(WorkflowError::OrderingViolation(format!(
                    "complete step {} ({}) first",
                    i + 1,
                    template.steps[i].label
                )));
            }
        }
        self.run.done[idx] = true;
        Ok(())
    }

    pub fn uncheck(&mut self, step: u32) -> Result<(), WorkflowError> {
        let template = self.require_active_template()?;
        let idx = validate_step_index(step, template.steps.len())?;
        self.run.done[idx] = false;
        Ok(())
    }

    pub fn skip(&mut self, step: u32) -> Result<(), WorkflowError> {
        let template = self.require_active_template()?;
        let idx = validate_step_index(step, template.steps.len())?;
        self.run.done[idx] = true;
        Ok(())
    }

    pub fn reset(&mut self) {
        // A bound engine cannot return to template selection: reset only clears
        // step progress for the assigned template, keeping it active.
        if self.bound {
            let steps = self.active_template().map(|t| t.steps.len());
            if let Some(steps) = steps {
                self.run.done = vec![false; steps];
            }
            return;
        }
        self.run = WorkflowRun::default();
    }

    pub fn set_issue(&mut self, number: u32, title: String) {
        self.run.active_issue = Some((number, truncate_issue_title(title)));
    }

    pub fn clear_issue(&mut self) {
        self.run.active_issue = None;
    }

    pub fn progress(&self) -> WorkflowProgress {
        self.progress_for_done(self.run.done.iter().filter(|d| **d).count() as u32)
    }

    fn visible_progress(&self) -> WorkflowProgress {
        let visible_done = self
            .run
            .done
            .iter()
            .take(self.visible_step_count())
            .filter(|done| **done)
            .count() as u32;
        self.progress_for_done(visible_done)
    }

    fn progress_for_done(&self, done: u32) -> WorkflowProgress {
        let total = self.run.done.len() as u32;
        WorkflowProgress {
            done,
            total,
            percent: done
                .checked_mul(100)
                .and_then(|value| value.checked_div(total))
                .unwrap_or(0),
        }
    }

    pub fn current_step(&self) -> Option<WorkflowStepStatus> {
        let template = self.active_template()?;
        let idx = self.run.done.iter().position(|d| !*d)?;
        Some(status_for_step(template, idx, self.run.done[idx]))
    }

    /// Step handoff appended to every step-state-changing tool result
    /// (`select_template`/`check`/`skip`/`uncheck`, #1113 AC2): the current
    /// step's focus block plus the progress and active-issue context the
    /// retired per-turn system prompt used to carry — the tool result is the
    /// model's immediate replacement channel for all three. Renders the step
    /// through [`step_focus_text`], the same function behind the
    /// idle-boundary nudges, so the two channels cannot drift apart.
    pub fn step_handoff_text(&self, heading: &str) -> String {
        let mut out = match self.current_step() {
            Some(step) => format!("\n{}", step_focus_text(&step, heading)),
            None => "\nAll workflow steps complete.".to_string(),
        };
        let progress = self.visible_progress();
        out.push_str(&format!(
            "\nProgress: {}/{} steps complete.",
            progress.done, progress.total
        ));
        if let Some((number, title)) = &self.run.active_issue {
            out.push_str(&format!("\nActive issue: #{number} — {title}"));
        }
        out
    }

    pub fn status_text(&self) -> String {
        match self.mode() {
            WorkflowMode::SelectingTemplate => self.selector_status_text(),
            WorkflowMode::Active | WorkflowMode::Complete => self.active_status_text(),
        }
    }

    /// Idle-boundary nudge for an auto-continuing workflow.
    ///
    /// With the system prompt static for the whole session (#1113), this
    /// nudge is the idle-boundary channel for workflow state: in `Active`
    /// mode it carries the current step's label and guidance; in
    /// `SelectingTemplate` mode (armed via [`Self::set_selector_nudge`],
    /// i.e. an explicit `--workflow` session) it presents the template
    /// selector instead of any system-prompt injection.
    pub fn auto_continue_nudge(&self) -> Option<String> {
        // No mandated status reply here: literal instruction-following models
        // treated the old "Respond with just the word DONE" sentence as a
        // status poll, answering without tool calls — a no-progress turn that
        // silently killed auto-continue for the rest of the run.
        const AUTO_CONTINUE_NUDGE: &str = "Workflow incomplete. Continue with the next incomplete step. Use the workflow tool to check off steps as you complete them. If a tool call failed, retry or work around it; if genuinely blocked, state which step is blocked and why. Never ask for permission or stop for any other reason than the task is entirely complete.";

        match self.auto_continue_target()? {
            NudgeTarget::Selector => Some(self.selector_nudge_text()),
            NudgeTarget::Step(step) => Some(format!(
                "{AUTO_CONTINUE_NUDGE}\n{}",
                step_focus_text(&step, "Current step")
            )),
        }
    }

    /// Corrective variant of [`Self::auto_continue_nudge`], sent when the
    /// PREVIOUS nudged turn made no progress: literal instruction-following
    /// models reply to the standard nudge with a bare status message and no
    /// tool calls, so a verbatim repeat just re-elicits the same stall.
    ///
    /// Lives here (not in the interface layer) so ALL workflow nudge wording
    /// is owned by the engine and covered by the same wording tests — a
    /// wording pass over the sibling nudges cannot miss this one. The
    /// dispatch loop requires BOTH wordings, so this yields `Some` in every
    /// state the standard nudge does — including selector mode (#1113).
    pub fn corrective_nudge(&self) -> Option<String> {
        const CORRECTIVE_NUDGE: &str = "Your last reply did not advance the workflow. If the current step is finished, check it off with the workflow tool now; otherwise continue working on it. Do not reply with only a status message.";

        match self.auto_continue_target()? {
            NudgeTarget::Selector => Some(format!(
                "Your last reply did not select a workflow template. {}",
                self.selector_nudge_text()
            )),
            NudgeTarget::Step(step) => Some(format!(
                "{CORRECTIVE_NUDGE}\n{}",
                step_focus_text(&step, "Current step")
            )),
        }
    }

    /// Shared gate for the auto-continue nudges: what the nudge should point
    /// the model at. Active-step continuation requires auto-continue; the
    /// template selector does NOT — it fires whenever the selector nudge is
    /// armed and no template is selected yet (#1113 AC3). The retired
    /// system-prompt selector never depended on `workflow.auto_continue`, and
    /// the nudge is now the sole proactive selection channel, so gating it on
    /// that setting would leave an `auto_continue: false` `--workflow`
    /// session with no way to learn it must select a template.
    fn auto_continue_target(&self) -> Option<NudgeTarget> {
        match self.mode() {
            WorkflowMode::SelectingTemplate if self.selector_nudge => Some(NudgeTarget::Selector),
            WorkflowMode::SelectingTemplate | WorkflowMode::Complete => None,
            WorkflowMode::Active if self.auto_continue => {
                self.current_step().map(NudgeTarget::Step)
            }
            WorkflowMode::Active => None,
        }
    }

    /// Template-selector wording pushed at the first idle boundary of a
    /// `--workflow` session that has not selected a template (#1113 AC3) —
    /// the selector reaches the model through the nudge channel, never
    /// through injected system-prompt text.
    ///
    /// This is the sole proactive selection channel, so it must carry
    /// everything the retired system-prompt selector carried: the active
    /// issue (when set) and the operator-configured
    /// `workflow.selector_prompt` — otherwise that config knob is silently
    /// dead in the exact flow it was designed for.
    fn selector_nudge_text(&self) -> String {
        let mut out = String::from(
            "No workflow template is selected. Choose the best template for this task now: call workflow(action=\"select_template\", template=\"<id>\").\n",
        );
        if let Some((num, title)) = &self.run.active_issue {
            out.push_str(&format!("Active issue: #{} — {}\n", num, title));
        }
        if let Some(prompt) = &self.selector_prompt {
            out.push_str(&format!("{}\n", prompt));
        }
        out.push_str("Available templates:\n");
        for t in self.list_templates() {
            out.push_str(&format!("- {} — {}: {}\n", t.id, t.label, t.description));
        }
        out
    }

    pub fn completion_nudge(&self) -> Option<String> {
        if !self.completion_nudge || self.mode() != WorkflowMode::Complete {
            return None;
        }
        let template = self.active_template()?;
        // The master agent now drives issue selection; on completion an agent
        // reports its result and stops rather than self-selecting a new issue.
        // A bound agent runs exactly one assigned workflow.
        if self.bound {
            return Some(format!(
                "All workflow steps complete for assigned template '{}' ({} steps). The assigned task is done — report your result and stop.",
                template.label,
                template.steps.len()
            ));
        }
        Some(format!(
            "All workflow steps complete for template '{}' ({} steps). The task is done — report your result and stop.",
            template.label,
            template.steps.len()
        ))
    }

    pub fn check_guards(&self) -> Result<(), WorkflowError> {
        self.check_matching_guards(|_| true)
    }

    pub fn check_matching_guards<F>(&self, mut applies: F) -> Result<(), WorkflowError>
    where
        F: FnMut(&WorkflowGuardRule) -> bool,
    {
        if !self.guards_enabled {
            return Ok(());
        }
        let template = self.require_active_template()?;
        for guard in template.guards.iter().filter(|guard| applies(guard)) {
            self.check_guard_rule(template, guard)?;
        }
        Ok(())
    }

    fn check_guard_rule(
        &self,
        template: &WorkflowTemplate,
        guard: &WorkflowGuardRule,
    ) -> Result<(), WorkflowError> {
        let idx = template
            .steps
            .iter()
            .position(|s| s.key == guard.before_step_key)
            .ok_or_else(|| {
                WorkflowError::InvalidConfig(format!(
                    "guard references unknown step key '{}' in template '{}'",
                    guard.before_step_key, template.id
                ))
            })?;
        for prior_idx in 0..idx {
            if !self.run.done[prior_idx] {
                return Err(WorkflowError::GuardBlocked(format!(
                    "{} Complete step {} ({}) first.",
                    guard.message,
                    prior_idx + 1,
                    template.steps[prior_idx].label
                )));
            }
        }
        Ok(())
    }

    pub fn active_template(&self) -> Option<&WorkflowTemplate> {
        self.run
            .template_index
            .and_then(|idx| self.templates.get(idx))
    }

    pub fn snapshot(&self, enabled: bool) -> WorkflowSnapshot {
        let active_template = self.active_template().map(summary_for_template);
        let steps = self
            .active_template()
            .map(|t| {
                t.steps
                    .iter()
                    .enumerate()
                    .filter(|(idx, _)| self.is_visible_step_index(*idx))
                    .map(|(idx, _)| status_for_step(t, idx, self.is_step_done_index(idx)))
                    .collect()
            })
            .unwrap_or_default();
        WorkflowSnapshot {
            enabled,
            guards_enabled: enabled && self.guards_enabled,
            mode: self.mode(),
            active_template,
            active_issue: self.run.active_issue.clone(),
            progress: self.visible_progress(),
            current_step: self.current_step(),
            steps,
            available_templates: self.list_templates(),
        }
    }

    fn selector_status_text(&self) -> String {
        let mut out = String::from("## Workflow Template Selection\n");
        if let Some((num, title)) = &self.run.active_issue {
            out.push_str(&format!("Active issue: #{} — {}\n", num, title));
        } else {
            out.push_str("Active issue: (not set)\n");
        }
        if let Some(prompt) = &self.selector_prompt {
            out.push_str(&format!("{}\n", prompt));
        } else {
            out.push_str(
                "Choose the best workflow template for this task before checking steps.\n",
            );
        }
        out.push_str("\nAvailable templates:\n");
        for t in self.list_templates() {
            out.push_str(&format!("- {} — {}: {}\n", t.id, t.label, t.description));
        }
        out.push_str("\nCall workflow(action=\"select_template\", template=\"<id>\") to begin.\n");
        out
    }

    fn active_status_text(&self) -> String {
        let template = match self.active_template() {
            Some(t) => t,
            None => return self.selector_status_text(),
        };
        let progress = self.visible_progress();
        let mode = self.mode();
        let mut out = format!(
            "## Active Workflow\nTemplate: {} ({})\nProgress: {}/{}\n",
            template.label, template.id, progress.done, progress.total
        );
        if let Some((num, title)) = &self.run.active_issue {
            out.push_str(&format!("Active issue: #{} — {}\n", num, title));
        } else {
            out.push_str("Active issue: (not set)\n");
        }
        for (idx, step) in template.steps.iter().enumerate() {
            let done = self.is_step_done_index(idx);
            // Status is an agent-control surface, not a full lookahead plan:
            // show contiguous completed history plus the current step only.
            // Future steps (even if skipped/done out of order) and their
            // guidance stay hidden until they become part of visible progress,
            // so agents cannot act on later-step instructions prematurely.
            if !self.is_visible_step_index(idx) {
                continue;
            }
            if done {
                out.push_str(&format!("  [✓] {}. {}\n", idx + 1, step.label));
                continue;
            }
            out.push_str(&format!(
                "CURRENT STEP → {}. {} [{}]\n",
                idx + 1,
                step.label,
                phase_display_name(&step.phase)
            ));
            if let Some(g) = &step.guidance {
                out.push_str(&format!("      Guidance: {}\n", g));
            }
        }
        if mode == WorkflowMode::Complete {
            out.push_str("\n✓ All workflow steps complete.\n");
        }
        if self.guards_enabled && !template.guards.is_empty() {
            let visible_guards: Vec<_> = template
                .guards
                .iter()
                .filter(|g| self.is_visible_step_key(&g.before_step_key))
                .collect();
            if !visible_guards.is_empty() {
                out.push_str("\nGuards:\n");
                for g in visible_guards {
                    out.push_str(&format!(
                        "- {} (before step key '{}')\n",
                        g.message, g.before_step_key
                    ));
                }
            }
        }
        out
    }

    fn is_step_done_index(&self, idx: usize) -> bool {
        *self.run.done.get(idx).unwrap_or(&false)
    }

    fn visible_step_count(&self) -> usize {
        let Some(template) = self.active_template() else {
            return 0;
        };
        match self.mode() {
            WorkflowMode::SelectingTemplate => 0,
            WorkflowMode::Complete => template.steps.len(),
            WorkflowMode::Active => self
                .run
                .done
                .iter()
                .take(template.steps.len())
                .position(|done| !*done)
                .map(|idx| idx + 1)
                .unwrap_or(template.steps.len()),
        }
    }

    fn is_visible_step_index(&self, idx: usize) -> bool {
        idx < self.visible_step_count()
    }

    fn is_visible_step_key(&self, key: &str) -> bool {
        self.active_template()
            .and_then(|template| template.steps.iter().position(|step| step.key == key))
            .is_some_and(|idx| self.is_visible_step_index(idx))
    }

    pub fn all_step_statuses(&self) -> Vec<WorkflowStepStatus> {
        self.active_template()
            .map(|template| {
                template
                    .steps
                    .iter()
                    .enumerate()
                    .map(|(idx, _)| status_for_step(template, idx, self.is_step_done_index(idx)))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn require_active_template(&self) -> Result<&WorkflowTemplate, WorkflowError> {
        self.active_template().ok_or_else(|| {
            WorkflowError::NoActiveTemplate(
                "no active workflow template: call workflow(action=\"select_template\", template=\"<id>\") first".into(),
            )
        })
    }
}

/// What an auto-continue nudge should point the model at: the template
/// selector (no template chosen yet) or the current incomplete step.
enum NudgeTarget {
    Selector,
    Step(WorkflowStepStatus),
}

/// Focus block naming a step — and its guidance, if any — under the given
/// heading (e.g. "Current step", "Next step"), so idle-boundary nudges and
/// workflow tool results carry the state that no longer lives in the system
/// prompt (#1113 AC2/AC4). This is the single wording source for the step
/// handoff: the workflow tool renders its `select_template`/`check`/`skip`/
/// `uncheck` handoffs through this same function so the two channels cannot
/// drift apart.
pub fn step_focus_text(step: &WorkflowStepStatus, heading: &str) -> String {
    let mut out = format!(
        "{} {}: {} [{}]",
        heading,
        step.index,
        step.label,
        phase_display_name(&step.phase)
    );
    if let Some(g) = &step.guidance {
        out.push_str(&format!("\nGuidance: {g}"));
    }
    out
}

fn summary_for_template(template: &WorkflowTemplate) -> WorkflowTemplateSummary {
    WorkflowTemplateSummary {
        id: template.id.clone(),
        label: template.label.clone(),
        description: template.description.clone(),
        when_to_use: template.when_to_use.clone(),
    }
}

fn status_for_step(template: &WorkflowTemplate, idx: usize, done: bool) -> WorkflowStepStatus {
    let step = &template.steps[idx];
    WorkflowStepStatus {
        index: (idx + 1) as u32,
        key: step.key.clone(),
        label: step.label.clone(),
        phase: step.phase.clone(),
        done,
        guidance: step.guidance.clone(),
    }
}

fn validate_step_index(step: u32, len: usize) -> Result<usize, WorkflowError> {
    if step == 0 || step as usize > len {
        return Err(WorkflowError::InvalidStep(format!(
            "invalid step {}: must be 1-{}",
            step, len
        )));
    }
    Ok((step - 1) as usize)
}

fn validate_templates(templates: &[WorkflowTemplate]) -> Result<(), WorkflowError> {
    use std::collections::HashSet;
    if templates.len() > MAX_TEMPLATE_COUNT {
        return Err(WorkflowError::InvalidConfig(format!(
            "too many workflow templates: {} > {}",
            templates.len(),
            MAX_TEMPLATE_COUNT
        )));
    }
    let mut ids = HashSet::new();
    for template in templates {
        if template.id.trim().is_empty() {
            return Err(WorkflowError::InvalidConfig(
                "template id cannot be empty".into(),
            ));
        }
        if !ids.insert(template.id.clone()) {
            return Err(WorkflowError::InvalidConfig(format!(
                "duplicate template id: {}",
                template.id
            )));
        }
        if template.steps.is_empty() {
            return Err(WorkflowError::InvalidConfig(format!(
                "template '{}' has no steps",
                template.id
            )));
        }
        if template.steps.len() > MAX_STEPS_PER_TEMPLATE {
            return Err(WorkflowError::InvalidConfig(format!(
                "template '{}' has too many steps: {} > {}",
                template.id,
                template.steps.len(),
                MAX_STEPS_PER_TEMPLATE
            )));
        }
        let mut step_keys = HashSet::new();
        for step in &template.steps {
            if step.key.trim().is_empty() {
                return Err(WorkflowError::InvalidConfig(format!(
                    "template '{}' has a step with empty key",
                    template.id
                )));
            }
            if !step_keys.insert(step.key.clone()) {
                return Err(WorkflowError::InvalidConfig(format!(
                    "template '{}' has duplicate step key '{}'",
                    template.id, step.key
                )));
            }
        }
        for guard in &template.guards {
            if !step_keys.contains(&guard.before_step_key) {
                return Err(WorkflowError::InvalidConfig(format!(
                    "template '{}' guard references unknown step key '{}'",
                    template.id, guard.before_step_key
                )));
            }
        }
    }
    Ok(())
}

fn truncate_issue_title(title: String) -> String {
    if title.len() <= MAX_ISSUE_TITLE_LEN {
        return title;
    }
    let mut end = MAX_ISSUE_TITLE_LEN;
    while end > 0 && !title.is_char_boundary(end) {
        end -= 1;
    }
    title[..end].to_string()
}

mod templates;
pub use templates::default_templates;
use templates::phase_display_name;
