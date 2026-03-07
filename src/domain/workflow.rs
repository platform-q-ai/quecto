//! Workflow tracking types for BDD/TDD development process.
//!
//! Pure domain types with no I/O dependencies. The workflow state tracks
//! which steps are done, the active issue, and provides progress reporting.

use serde::{Deserialize, Serialize};

/// Maximum allowed length for issue titles (chars).
/// Prevents unbounded memory/context-token usage from excessively long titles.
const MAX_ISSUE_TITLE_LEN: usize = 500;

/// Maximum number of steps allowed in a workflow configuration.
/// Prevents DoS via config files with millions of steps.
const MAX_STEPS: usize = 100;

/// A single step in the development workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStep {
    pub id: u32,
    pub label: String,
    pub phase: String,
}

/// The in-memory state of a workflow cycle.
#[derive(Debug, Clone)]
pub struct WorkflowState {
    steps: Vec<WorkflowStep>,
    done: Vec<bool>,
    active_issue: Option<(u32, String)>,
}

/// Progress summary for a workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowProgress {
    pub done: u32,
    pub total: u32,
    pub percent: u32,
}

/// Snapshot of workflow state for serialization by the infrastructure layer.
/// Keeps `serde_json` out of the domain layer.
#[derive(Debug, Clone)]
pub struct WorkflowStateSnapshot {
    pub steps: Vec<(WorkflowStep, bool)>,
    pub progress: WorkflowProgress,
    pub active_issue: Option<(u32, String)>,
}

/// Error type for workflow operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowError {
    /// Step number is out of range (1-based).
    InvalidStep(String),
    /// Step ordering not satisfied.
    OrderingViolation(String),
    /// Commit blocked because required steps are incomplete.
    CommitBlocked(String),
}

impl std::fmt::Display for WorkflowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkflowError::InvalidStep(msg) => write!(f, "{}", msg),
            WorkflowError::OrderingViolation(msg) => write!(f, "{}", msg),
            WorkflowError::CommitBlocked(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for WorkflowError {}

impl WorkflowState {
    /// Create a new workflow state from the given steps.
    ///
    /// Clamps to [`MAX_STEPS`] to prevent unbounded allocations from
    /// malicious config files.
    pub fn new(steps: Vec<WorkflowStep>) -> Self {
        let steps: Vec<WorkflowStep> = if steps.len() > MAX_STEPS {
            steps.into_iter().take(MAX_STEPS).collect()
        } else {
            steps
        };
        let len = steps.len();
        Self {
            steps,
            done: vec![false; len],
            active_issue: None,
        }
    }

    /// Create a workflow state from config.
    pub fn from_config(config: &WorkflowConfig) -> Self {
        Self::new(config.steps.clone())
    }

    /// Create the 16-step BDD/TDD workflow (test-only convenience).
    #[cfg(any(test, feature = "test-support"))]
    pub fn default_bdd() -> Self {
        Self::new(bdd_steps())
    }

    /// Return all steps.
    pub fn steps(&self) -> &[WorkflowStep] {
        &self.steps
    }

    /// Return the done flags.
    pub fn done_flags(&self) -> &[bool] {
        &self.done
    }

    /// Check whether a specific step is done.
    pub fn is_done(&self, step: u32) -> Result<bool, WorkflowError> {
        let idx = self.validate_step(step)?;
        Ok(self.done[idx])
    }

    /// Mark a step as done, enforcing ordering (all previous steps must be done).
    pub fn check(&mut self, step: u32) -> Result<(), WorkflowError> {
        let idx = self.validate_step(step)?;

        // Enforce ordering: all previous steps must be done.
        for i in 0..idx {
            if !self.done[i] {
                return Err(WorkflowError::OrderingViolation(format!(
                    "complete step {} first",
                    self.steps[i].id
                )));
            }
        }

        self.done[idx] = true;
        Ok(())
    }

    /// Unmark a step.
    ///
    /// This is a low-level operation that may create ordering gaps (e.g.
    /// unchecking step 2 while steps 3+ remain checked). Use `reset()` for
    /// a clean restart. The ordering invariant is re-enforced by `check()`.
    pub fn uncheck(&mut self, step: u32) -> Result<(), WorkflowError> {
        let idx = self.validate_step(step)?;
        self.done[idx] = false;
        Ok(())
    }

    /// Force-mark a step as done regardless of ordering.
    pub fn skip(&mut self, step: u32) -> Result<(), WorkflowError> {
        let idx = self.validate_step(step)?;
        self.done[idx] = true;
        Ok(())
    }

    /// Reset all steps and clear the active issue.
    pub fn reset(&mut self) {
        self.done.fill(false);
        self.active_issue = None;
    }

    /// Record the active issue.
    ///
    /// Titles are truncated to [`MAX_ISSUE_TITLE_LEN`] characters to
    /// prevent unbounded system prompt / memory growth.
    pub fn set_issue(&mut self, number: u32, title: String) {
        let title = if title.len() > MAX_ISSUE_TITLE_LEN {
            // Truncate at char boundary
            let mut end = MAX_ISSUE_TITLE_LEN;
            while !title.is_char_boundary(end) && end > 0 {
                end -= 1;
            }
            title[..end].to_string()
        } else {
            title
        };
        self.active_issue = Some((number, title));
    }

    /// Clear the active issue.
    pub fn clear_issue(&mut self) {
        self.active_issue = None;
    }

    /// Return the active issue, if set.
    pub fn active_issue(&self) -> Option<&(u32, String)> {
        self.active_issue.as_ref()
    }

    /// Compute progress.
    pub fn progress(&self) -> WorkflowProgress {
        let total = self.done.len() as u32;
        let done = self.done.iter().filter(|&&d| d).count() as u32;
        let percent = if total > 0 { (done * 100) / total } else { 0 };
        WorkflowProgress {
            done,
            total,
            percent,
        }
    }

    /// Build a human-readable system prompt snippet for the current state.
    ///
    /// Groups steps by their `phase` field (derived from step data, not
    /// hardcoded IDs) so custom step configurations display correctly.
    pub fn system_prompt_snippet(&self) -> String {
        let progress = self.progress();
        let mut out = format!(
            "## Active Development Workflow (Quecto README.md)\n\
             Progress: {}/{} steps complete.\n",
            progress.done, progress.total
        );

        if let Some((num, title)) = &self.active_issue {
            out.push_str(&format!("Active issue: #{} — {}\n", num, title));
        } else {
            out.push_str(
                "Active issue: (not set) — call workflow(action=\"set_issue\", issueNumber=<n>, issueTitle=\"...\")\n",
            );
        }

        // Find current step (first unchecked)
        let current_step = self.done.iter().position(|&d| !d);

        // Group consecutive steps by phase, using display names derived from
        // the phase field rather than hardcoded step IDs.
        let mut groups: Vec<(&str, Vec<(usize, &WorkflowStep)>)> = Vec::new();
        for (idx, step) in self.steps.iter().enumerate() {
            let display_phase = phase_display_name(&step.phase);
            if groups
                .last()
                .is_some_and(|(name, _)| *name == display_phase)
            {
                groups.last_mut().unwrap().1.push((idx, step));
            } else {
                groups.push((display_phase, vec![(idx, step)]));
            }
        }

        for (phase_name, step_entries) in &groups {
            out.push_str(&format!("\n[{}]\n", phase_name));
            for &(idx, step) in step_entries {
                let marker = if self.done[idx] { "✓" } else { " " };
                let is_current = current_step == Some(idx);
                if is_current {
                    out.push_str(&format!(
                        "CURRENT STEP → {}. {} [{}]\n",
                        step.id, step.label, phase_name
                    ));
                } else {
                    out.push_str(&format!("  [{}] {}. {}\n", marker, step.id, step.label));
                }
            }
        }

        if progress.done >= progress.total && progress.total > 0 {
            out.push_str("\n✓ All steps complete! You may start a new workflow cycle with `workflow reset`.\n");
        }

        out
    }

    /// Build a system prompt snippet that also includes guard rule reminders.
    pub fn system_prompt_snippet_with_guards(&self, guards: &[GuardRule]) -> String {
        let mut snippet = self.system_prompt_snippet();
        for rule in guards {
            let cmds = rule.commands.join(", ");
            snippet.push_str(&format!(
                "\n⚠ Guard: `{}` blocked until steps 1–{} are checked. {}\n",
                cmds,
                rule.before_step.saturating_sub(1),
                rule.message
            ));
        }
        snippet
    }

    /// Generate an auto-continue nudge message, if there are incomplete steps.
    ///
    /// Returns `None` when all steps are complete (no nudge needed).
    /// Returns `Some(message)` with the next incomplete step to work on.
    pub fn auto_continue_nudge(&self) -> Option<String> {
        // position() returns None when all steps are done — no need for
        // a separate progress() call.
        let next_idx = self.done.iter().position(|&d| !d)?;
        let step = &self.steps[next_idx];
        Some(format!(
            "Continue the workflow — next incomplete step is step {} ({}). \
             Proceed with this step now, then call workflow(action=\"check\", step={}).",
            step.id, step.label, step.id
        ))
    }

    /// Generate a completion nudge message when all steps are done.
    ///
    /// Returns `None` when steps are still incomplete.
    /// Returns `Some(message)` prompting the agent to close the issue and pick the next.
    pub fn completion_nudge(&self) -> Option<String> {
        let progress = self.progress();
        if progress.done < progress.total {
            return None;
        }
        Some(
            "All steps complete! You have completed all 16 workflow steps for this issue. \
             Please now:\n\
             1. Close the current issue (if applicable)\n\
             2. Pick the next open issue — if no open issues exist, respond with just the word NONE\n\
             3. Record it: call the workflow tool with action=\"set_issue\", issueNumber=<n>, issueTitle=\"...\"\n\
             4. Reset the checklist: call the workflow tool with action=\"reset\"\n\
             5. Begin Step 1 immediately for the new issue"
                .to_string(),
        )
    }

    /// Check whether `git commit` is allowed given the enforcement threshold.
    ///
    /// - `enforce_commit_after_step = None` → always allowed (enforcement disabled)
    /// - `enforce_commit_after_step = Some(0)` → always allowed
    /// - `enforce_commit_after_step = Some(n)` → all steps 1..=n must be checked
    ///
    /// Returns `Ok(())` if allowed, `Err(WorkflowError::CommitBlocked)` if blocked.
    pub fn check_commit_allowed(
        &self,
        enforce_commit_after_step: Option<u32>,
    ) -> Result<(), WorkflowError> {
        self.check_steps_complete(enforce_commit_after_step.unwrap_or(0))
    }

    /// Check whether all steps before `before_step` are completed.
    ///
    /// - `before_step = 0` → always allowed
    /// - `before_step = n` → all steps 1..=(n-1) must be checked
    ///
    /// Returns `Ok(())` if allowed, `Err(WorkflowError::CommitBlocked)` if blocked.
    pub fn check_steps_complete(&self, before_step: u32) -> Result<(), WorkflowError> {
        if before_step == 0 {
            return Ok(());
        }
        let threshold = before_step.saturating_sub(1);
        let limit = std::cmp::min(threshold as usize, self.steps.len());
        for i in 0..limit {
            if !self.done[i] {
                return Err(WorkflowError::CommitBlocked(format!(
                    "blocked: complete step {} ({}) first. \
                     Steps 1–{} must be checked.",
                    self.steps[i].id, self.steps[i].label, threshold
                )));
            }
        }
        Ok(())
    }

    /// Serialize the dynamic state (done flags + active issue) for persistence.
    /// Step definitions come from config and are not persisted.
    pub fn to_persistable(&self) -> WorkflowPersistable {
        WorkflowPersistable {
            done: self.done.clone(),
            active_issue: self.active_issue.clone(),
        }
    }

    /// Restore state from a persisted snapshot with explicit step definitions.
    pub fn from_persistable_with_steps(
        p: &WorkflowPersistable,
        steps: Option<Vec<WorkflowStep>>,
    ) -> Self {
        let steps = steps.unwrap_or_else(default_steps);
        let len = steps.len();
        let mut done = p.done.clone();
        // resize handles both padding (len > done.len()) and truncation.
        done.resize(len, false);
        Self {
            steps,
            done,
            active_issue: p.active_issue.clone(),
        }
    }

    /// Create a snapshot of the current state for event serialization.
    /// The infrastructure layer converts this to JSON — keeps `serde_json`
    /// out of the domain layer.
    pub fn snapshot(&self) -> WorkflowStateSnapshot {
        WorkflowStateSnapshot {
            steps: self
                .steps
                .iter()
                .zip(self.done.iter())
                .map(|(s, &d)| (s.clone(), d))
                .collect(),
            progress: self.progress(),
            active_issue: self.active_issue.clone(),
        }
    }

    /// Validate a step number (1-based) and return the 0-based index.
    fn validate_step(&self, step: u32) -> Result<usize, WorkflowError> {
        if step == 0 || step as usize > self.steps.len() {
            return Err(WorkflowError::InvalidStep(format!(
                "invalid step {}: must be 1-{}",
                step,
                self.steps.len()
            )));
        }
        Ok((step - 1) as usize)
    }
}

/// Map a phase field value to its display name.
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

/// Serializable snapshot of workflow state for session persistence.
///
/// Contains only the dynamic state (done flags + active issue) — the step
/// definitions come from config and are not persisted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowPersistable {
    pub done: Vec<bool>,
    pub active_issue: Option<(u32, String)>,
}

/// A guard rule that blocks specific commands before a workflow step.
///
/// **Note:** This is a developer convenience, NOT a security boundary.
/// Any user with config.json write access can modify or remove guards.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardRule {
    /// Command patterns to match, e.g. `["git commit", "git push"]`.
    /// Each pattern is `<binary> <subcommand>` — matched against bash
    /// tool arguments with flag-skipping and subshell detection.
    pub commands: Vec<String>,
    /// Block until all steps up to (but not including) this step number
    /// are completed. E.g. `before_step: 7` blocks until steps 1-6 are done.
    pub before_step: u32,
    /// Custom message returned to the LLM when the command is blocked.
    pub message: String,
}

/// Workflow configuration section for config.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_steps", skip_serializing_if = "is_default_steps")]
    pub steps: Vec<WorkflowStep>,
    /// When true, after each agent run completes with incomplete steps, the
    /// system injects a nudge message to continue with the next step.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub auto_continue: bool,
    /// When true, after all steps are checked, the system prompts the agent
    /// to close the current issue and pick the next one.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub completion_nudge: bool,
    /// Guard rules that block specific commands before specific workflow steps.
    /// Empty by default — no commands are blocked unless configured.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guards: Vec<GuardRule>,

    // ── Deprecated fields (backward compat) ─────────────────────────────
    // Silently consumed during deserialization to avoid "unknown field" errors
    // on existing config files. Not used — replaced by `guards`.
    #[serde(default, skip_serializing)]
    #[doc(hidden)]
    pub guard_commit: Option<bool>,
    #[serde(default, skip_serializing)]
    #[doc(hidden)]
    pub enforce_commit_after_step: Option<u32>,
}

impl Default for WorkflowConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            steps: default_steps(),
            auto_continue: true,
            completion_nudge: true,
            guards: vec![],
            guard_commit: None,
            enforce_commit_after_step: None,
        }
    }
}

impl WorkflowConfig {
    /// Migrate deprecated `guard_commit`/`enforce_commit_after_step` to `guards`.
    /// Logs a warning if deprecated fields are detected.
    /// Call this after deserialization to ensure backward compatibility.
    pub fn migrate_deprecated(&mut self) {
        if self.guards.is_empty() && self.guard_commit == Some(true) {
            let step = self.enforce_commit_after_step.unwrap_or(6);
            self.guards.push(GuardRule {
                commands: vec!["git commit".into(), "git push".into()],
                before_step: step + 1,
                message: format!(
                    "Complete steps 1–{} before committing. \
                     (Migrated from deprecated guard_commit/enforce_commit_after_step config.)",
                    step
                ),
            });
            tracing::warn!(
                "Deprecated workflow config: 'guard_commit' and 'enforce_commit_after_step' \
                 have been replaced by 'guards'. Please update your config.json."
            );
        }
        // Clear deprecated fields after migration
        self.guard_commit = None;
        self.enforce_commit_after_step = None;
    }
}

/// Returns true if the steps match the default 16-step template.
/// Used by `skip_serializing_if` to avoid injecting default workflow
/// config into serialized config files on round-trip.
fn is_default_steps(steps: &[WorkflowStep]) -> bool {
    steps.is_empty()
}

fn default_true() -> bool {
    true
}

fn is_true(val: &bool) -> bool {
    *val
}

/// Default steps: empty. Steps must be configured explicitly via config.json.
pub fn default_steps() -> Vec<WorkflowStep> {
    vec![]
}

/// The 16-step BDD/TDD workflow matching README.md.
/// Test-only: used by `default_bdd()` as a reference template.
#[cfg(any(test, feature = "test-support"))]
pub fn bdd_steps() -> Vec<WorkflowStep> {
    vec![
        WorkflowStep {
            id: 1,
            label: "Update Scenarios / Add new features".into(),
            phase: "red".into(),
        },
        WorkflowStep {
            id: 2,
            label: "Write/update unit tests".into(),
            phase: "red".into(),
        },
        WorkflowStep {
            id: 3,
            label: "Ensure new/modified tests FAIL (RED)".into(),
            phase: "red".into(),
        },
        WorkflowStep {
            id: 4,
            label: "Implement code (GREEN)".into(),
            phase: "green".into(),
        },
        WorkflowStep {
            id: 5,
            label: "Refactor (perf, security, clean arch)".into(),
            phase: "refactor".into(),
        },
        WorkflowStep {
            id: 6,
            label: "Ensure tests still pass (GREEN)".into(),
            phase: "green".into(),
        },
        WorkflowStep {
            id: 7,
            label: "Commit".into(),
            phase: "ci_cd".into(),
        },
        WorkflowStep {
            id: 8,
            label: "Push".into(),
            phase: "ci_cd".into(),
        },
        WorkflowStep {
            id: 9,
            label: "Create PR".into(),
            phase: "ci_cd".into(),
        },
        WorkflowStep {
            id: 10,
            label: "Despatch reviewers (Arch, Security, Perf)".into(),
            phase: "review".into(),
        },
        WorkflowStep {
            id: 11,
            label: "Fix all valid review concerns".into(),
            phase: "review".into(),
        },
        WorkflowStep {
            id: 12,
            label: "Push changes to remote".into(),
            phase: "review".into(),
        },
        WorkflowStep {
            id: 13,
            label: "Reply to comments and mark resolved".into(),
            phase: "review".into(),
        },
        WorkflowStep {
            id: 14,
            label: "Run pre-merge hooks (real-LLM, machete, deny)".into(),
            phase: "ci_cd".into(),
        },
        WorkflowStep {
            id: 15,
            label: "Merge".into(),
            phase: "ci_cd".into(),
        },
        WorkflowStep {
            id: 16,
            label: "Move to local master and pull".into(),
            phase: "ci_cd".into(),
        },
    ]
}

#[cfg(test)]
#[path = "workflow_tests.rs"]
mod tests;
