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
        })
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

    pub fn select_template(
        &mut self,
        template_id: &str,
        issue: Option<(u32, String)>,
    ) -> Result<(), WorkflowError> {
        let template_index = self
            .templates
            .iter()
            .position(|t| t.id == template_id)
            .ok_or_else(|| {
                WorkflowError::UnknownTemplate(format!("unknown template: {}", template_id))
            })?;
        let template = &self.templates[template_index];
        let active_issue = issue
            .map(|(number, title)| (number, truncate_issue_title(title)))
            .or_else(|| self.run.active_issue.clone());
        self.run.template_id = Some(template.id.clone());
        self.run.template_index = Some(template_index);
        self.run.done = vec![false; template.steps.len()];
        self.run.active_issue = active_issue;
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
        self.run = WorkflowRun::default();
    }

    pub fn set_issue(&mut self, number: u32, title: String) {
        self.run.active_issue = Some((number, truncate_issue_title(title)));
    }

    pub fn clear_issue(&mut self) {
        self.run.active_issue = None;
    }

    pub fn progress(&self) -> WorkflowProgress {
        let total = self.run.done.len() as u32;
        let done = self.run.done.iter().filter(|d| **d).count() as u32;
        WorkflowProgress {
            done,
            total,
            percent: if total == 0 { 0 } else { (done * 100) / total },
        }
    }

    pub fn current_step(&self) -> Option<WorkflowStepStatus> {
        let template = self.active_template()?;
        let idx = self.run.done.iter().position(|d| !*d)?;
        Some(status_for_step(template, idx, self.run.done[idx]))
    }

    pub fn status_text(&self) -> String {
        match self.mode() {
            WorkflowMode::SelectingTemplate => self.selector_status_text(),
            WorkflowMode::Active | WorkflowMode::Complete => self.active_status_text(),
        }
    }

    pub fn prompt_snippet(&self) -> String {
        match self.mode() {
            WorkflowMode::SelectingTemplate => self.selector_prompt_text(),
            WorkflowMode::Active | WorkflowMode::Complete => self.active_prompt_text(),
        }
    }

    pub fn auto_continue_nudge(&self) -> Option<String> {
        if !self.auto_continue || self.mode() != WorkflowMode::Active {
            return None;
        }
        let step = self.current_step()?;
        Some(format!(
            "Continue the workflow — next incomplete step is step {} ({}) in the {} template. Proceed with this step now, then call workflow(action=\"check\", step={}).",
            step.index,
            step.label,
            self.active_template()?.label,
            step.index
        ))
    }

    pub fn completion_nudge(&self) -> Option<String> {
        if !self.completion_nudge || self.mode() != WorkflowMode::Complete {
            return None;
        }
        let template = self.active_template()?;
        Some(format!(
            "All workflow steps complete for template '{}' ({} steps). Close out the current issue if applicable, pick the next issue, call workflow(action=\"reset\") and then workflow(action=\"select_template\", template=\"<id>\").",
            template.label,
            template.steps.len()
        ))
    }

    pub fn check_guards(&self) -> Result<(), WorkflowError> {
        if !self.guards_enabled {
            return Ok(());
        }
        let template = self.require_active_template()?;
        for guard in &template.guards {
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
        }
        Ok(())
    }

    pub fn active_template(&self) -> Option<&WorkflowTemplate> {
        self.run
            .template_index
            .and_then(|idx| self.templates.get(idx))
    }

    pub fn template_guards(&self) -> &[WorkflowGuardRule] {
        self.active_template()
            .map(|t| t.guards.as_slice())
            .unwrap_or(&[])
    }

    pub fn snapshot(&self, enabled: bool) -> WorkflowSnapshot {
        let active_template = self.active_template().map(summary_for_template);
        let steps = self
            .active_template()
            .map(|t| {
                t.steps
                    .iter()
                    .enumerate()
                    .map(|(idx, _)| {
                        status_for_step(t, idx, *self.run.done.get(idx).unwrap_or(&false))
                    })
                    .collect()
            })
            .unwrap_or_default();
        WorkflowSnapshot {
            enabled,
            guards_enabled: enabled && self.guards_enabled,
            mode: self.mode(),
            active_template,
            active_issue: self.run.active_issue.clone(),
            progress: self.progress(),
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
        let progress = self.progress();
        let mode = self.mode();
        let current_idx = if mode == WorkflowMode::Complete {
            None
        } else {
            self.current_step().map(|step| step.index as usize)
        };
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
            let done = *self.run.done.get(idx).unwrap_or(&false);
            if !done && current_idx == Some(idx + 1) {
                out.push_str(&format!(
                    "CURRENT STEP → {}. {} [{}]\n",
                    idx + 1,
                    step.label,
                    phase_display_name(&step.phase)
                ));
                if let Some(g) = &step.guidance {
                    out.push_str(&format!("  Guidance: {}\n", g));
                }
            } else {
                out.push_str(&format!(
                    "  [{}] {}. {}\n",
                    if done { '✓' } else { ' ' },
                    idx + 1,
                    step.label
                ));
            }
        }
        if mode == WorkflowMode::Complete {
            out.push_str("\n✓ All workflow steps complete.\n");
        }
        if self.guards_enabled && !template.guards.is_empty() {
            out.push_str("\nGuards:\n");
            for g in &template.guards {
                out.push_str(&format!(
                    "- {} (before step key '{}')\n",
                    g.message, g.before_step_key
                ));
            }
        }
        out
    }

    fn selector_prompt_text(&self) -> String {
        let mut out = String::from("## Active Development Workflow\nMODE: SELECT TEMPLATE\n");
        if let Some((num, title)) = &self.run.active_issue {
            out.push_str(&format!("Active issue: #{} — {}\n", num, title));
        } else {
            out.push_str("Active issue: (not set)\n");
        }
        if let Some(prompt) = &self.selector_prompt {
            out.push_str(&format!("{}\n", prompt));
        } else {
            out.push_str(
                "Choose the best workflow template for this task before starting execution.\n",
            );
        }
        out.push_str("Available templates:\n");
        for t in self.list_templates() {
            out.push_str(&format!("- {}: {}\n", t.id, t.description));
        }
        out.push_str(
            "Call workflow(action=\"select_template\", template=\"<id>\") before checking steps.\n",
        );
        out
    }

    fn active_prompt_text(&self) -> String {
        let template = match self.active_template() {
            Some(t) => t,
            None => return self.selector_prompt_text(),
        };
        let progress = self.progress();
        let mut out = format!(
            "## Active Development Workflow\nTemplate: {} ({})\nProgress: {}/{} steps complete.\n",
            template.label, template.id, progress.done, progress.total
        );
        if let Some((num, title)) = &self.run.active_issue {
            out.push_str(&format!("Active issue: #{} — {}\n", num, title));
        } else {
            out.push_str("Active issue: (not set)\n");
        }
        if let Some(step) = self.current_step() {
            out.push_str(&format!(
                "CURRENT STEP → {}. {} [{}]\n",
                step.index,
                step.label,
                phase_display_name(&step.phase)
            ));
            if let Some(g) = step.guidance {
                out.push_str(&format!("Guidance: {}\n", g));
            }
        } else {
            out.push_str("✓ All workflow steps complete.\n");
        }
        if self.guards_enabled && !template.guards.is_empty() {
            for guard in &template.guards {
                out.push_str(&format!("Guard: {}\n", guard.message));
            }
        }
        out
    }

    fn require_active_template(&self) -> Result<&WorkflowTemplate, WorkflowError> {
        self.active_template().ok_or_else(|| {
            WorkflowError::NoActiveTemplate(
                "no active workflow template: call workflow(action=\"select_template\", template=\"<id>\") first".into(),
            )
        })
    }
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

pub fn default_templates() -> Vec<WorkflowTemplate> {
    vec![
        WorkflowTemplate {
            id: "feature".into(),
            label: "Feature".into(),
            description: "New capability or substantial behavior expansion.".into(),
            when_to_use: Some("Use for new user-facing or system-facing behavior.".into()),
            steps: vec![
                WorkflowTemplateStep {
                    key: "scenarios".into(),
                    label: "Update scenarios / add feature coverage".into(),
                    phase: "red".into(),
                    guidance: Some(
                        "Start by updating feature coverage and task-facing scenarios.".into(),
                    ),
                },
                WorkflowTemplateStep {
                    key: "tests".into(),
                    label: "Write/update tests".into(),
                    phase: "red".into(),
                    guidance: None,
                },
                WorkflowTemplateStep {
                    key: "red".into(),
                    label: "Ensure tests fail (RED)".into(),
                    phase: "red".into(),
                    guidance: None,
                },
                WorkflowTemplateStep {
                    key: "green".into(),
                    label: "Implement code (GREEN)".into(),
                    phase: "green".into(),
                    guidance: Some(
                        "Write the minimum code needed to satisfy the failing tests.".into(),
                    ),
                },
                WorkflowTemplateStep {
                    key: "refactor".into(),
                    label: "Refactor".into(),
                    phase: "refactor".into(),
                    guidance: None,
                },
                WorkflowTemplateStep {
                    key: "verify".into(),
                    label: "Ensure tests still pass".into(),
                    phase: "green".into(),
                    guidance: None,
                },
                WorkflowTemplateStep {
                    key: "commit".into(),
                    label: "Commit changes".into(),
                    phase: "ci_cd".into(),
                    guidance: None,
                },
            ],
            guards: vec![WorkflowGuardRule {
                commands: vec!["git commit".into(), "git push".into()],
                before_step_key: "commit".into(),
                message: "Complete implementation and verification steps before commit/push."
                    .into(),
            }],
        },
        WorkflowTemplate {
            id: "fix".into(),
            label: "Fix".into(),
            description: "Bug fix or regression correction.".into(),
            when_to_use: Some(
                "Use when behavior is broken and needs reproduction plus repair.".into(),
            ),
            steps: vec![
                WorkflowTemplateStep {
                    key: "repro".into(),
                    label: "Capture reproduction / failing scenario".into(),
                    phase: "red".into(),
                    guidance: Some(
                        "Start by reproducing the bug in a scenario or focused test.".into(),
                    ),
                },
                WorkflowTemplateStep {
                    key: "tests".into(),
                    label: "Write/update regression tests".into(),
                    phase: "red".into(),
                    guidance: None,
                },
                WorkflowTemplateStep {
                    key: "red".into(),
                    label: "Ensure regression test fails (RED)".into(),
                    phase: "red".into(),
                    guidance: None,
                },
                WorkflowTemplateStep {
                    key: "green".into(),
                    label: "Implement fix (GREEN)".into(),
                    phase: "green".into(),
                    guidance: None,
                },
                WorkflowTemplateStep {
                    key: "verify".into(),
                    label: "Verify regression is fixed".into(),
                    phase: "green".into(),
                    guidance: None,
                },
                WorkflowTemplateStep {
                    key: "commit".into(),
                    label: "Commit changes".into(),
                    phase: "ci_cd".into(),
                    guidance: None,
                },
            ],
            guards: vec![WorkflowGuardRule {
                commands: vec!["git commit".into(), "git push".into()],
                before_step_key: "commit".into(),
                message: "Complete reproduction, fix, and verification before commit/push.".into(),
            }],
        },
        WorkflowTemplate {
            id: "refactor".into(),
            label: "Refactor".into(),
            description: "Internal cleanup without intended behavioral change.".into(),
            when_to_use: Some(
                "Use for structural cleanup, performance, or design improvement.".into(),
            ),
            steps: vec![
                WorkflowTemplateStep {
                    key: "safety".into(),
                    label: "Define safety net tests".into(),
                    phase: "red".into(),
                    guidance: Some(
                        "Identify the tests that must stay green throughout the refactor.".into(),
                    ),
                },
                WorkflowTemplateStep {
                    key: "baseline".into(),
                    label: "Verify baseline behavior".into(),
                    phase: "green".into(),
                    guidance: None,
                },
                WorkflowTemplateStep {
                    key: "refactor".into(),
                    label: "Refactor internals".into(),
                    phase: "refactor".into(),
                    guidance: None,
                },
                WorkflowTemplateStep {
                    key: "verify".into(),
                    label: "Re-run safety net".into(),
                    phase: "green".into(),
                    guidance: None,
                },
                WorkflowTemplateStep {
                    key: "commit".into(),
                    label: "Commit changes".into(),
                    phase: "ci_cd".into(),
                    guidance: None,
                },
            ],
            guards: vec![WorkflowGuardRule {
                commands: vec!["git commit".into()],
                before_step_key: "commit".into(),
                message: "Complete the refactor safety net before commit.".into(),
            }],
        },
        WorkflowTemplate {
            id: "chore".into(),
            label: "Chore".into(),
            description: "Routine maintenance or configuration work.".into(),
            when_to_use: Some("Use for docs, dependency, CI, or maintenance tasks.".into()),
            steps: vec![
                WorkflowTemplateStep {
                    key: "scope".into(),
                    label: "Clarify the maintenance task".into(),
                    phase: "red".into(),
                    guidance: Some("Restate the task and identify any required validation.".into()),
                },
                WorkflowTemplateStep {
                    key: "change".into(),
                    label: "Apply the maintenance change".into(),
                    phase: "green".into(),
                    guidance: None,
                },
                WorkflowTemplateStep {
                    key: "verify".into(),
                    label: "Verify the change".into(),
                    phase: "green".into(),
                    guidance: None,
                },
                WorkflowTemplateStep {
                    key: "commit".into(),
                    label: "Commit changes".into(),
                    phase: "ci_cd".into(),
                    guidance: None,
                },
            ],
            guards: vec![WorkflowGuardRule {
                commands: vec!["git commit".into()],
                before_step_key: "commit".into(),
                message: "Verify the maintenance change before commit.".into(),
            }],
        },
    ]
}

fn phase_display_name(phase: &str) -> &str {
    match phase {
        "red" => "RED",
        "green" => "GREEN",
        "refactor" => "REFACTOR",
        "ci_cd" => "CI/CD",
        "review" => "REVIEW",
        other => other,
    }
}
