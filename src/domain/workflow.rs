//! Workflow tracking types for BDD/TDD development process.
//!
//! Pure domain types with no I/O dependencies. The workflow state tracks
//! which steps are done, the active issue, and provides progress reporting.

use serde::{Deserialize, Serialize};

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

/// Error type for workflow operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowError {
    /// Step number is out of range (1-based).
    InvalidStep(String),
    /// Step ordering not satisfied.
    OrderingViolation(String),
}

impl std::fmt::Display for WorkflowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkflowError::InvalidStep(msg) => write!(f, "{}", msg),
            WorkflowError::OrderingViolation(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for WorkflowError {}

impl WorkflowState {
    /// Create a new workflow state from the given steps.
    pub fn new(steps: Vec<WorkflowStep>) -> Self {
        let len = steps.len();
        Self {
            steps,
            done: vec![false; len],
            active_issue: None,
        }
    }

    /// Create a default 16-step BDD/TDD workflow.
    pub fn default_bdd() -> Self {
        Self::new(default_steps())
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
    pub fn set_issue(&mut self, number: u32, title: String) {
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
    pub fn system_prompt_snippet(&self) -> String {
        let progress = self.progress();
        let mut out = format!(
            "## Active Development Workflow (Quecto AGENTS.md)\n\
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

        // Group steps by phase for display
        let phases = [
            ("RED", vec![1, 2, 3]),
            ("GREEN", vec![4]),
            ("REFACTOR", vec![5]),
            ("GREEN", vec![6]),
            ("CI/CD", vec![7, 8, 9]),
            ("REVIEW", vec![10, 11, 12, 13]),
            ("CI/CD", vec![14, 15, 16]),
        ];

        for (phase_name, step_ids) in &phases {
            out.push_str(&format!("\n[{}]\n", phase_name));
            for &sid in step_ids {
                if let Some(idx) = self.steps.iter().position(|s| s.id == sid) {
                    let marker = if self.done[idx] { "✓" } else { " " };
                    let step = &self.steps[idx];
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
        }

        out
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

/// Workflow configuration section for config.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_steps")]
    pub steps: Vec<WorkflowStep>,
}

impl Default for WorkflowConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            steps: default_steps(),
        }
    }
}

fn default_true() -> bool {
    true
}

/// The default 16-step BDD/TDD workflow matching AGENTS.md.
pub fn default_steps() -> Vec<WorkflowStep> {
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

/// Serialize the workflow state to a JSON value for UDS event emission.
pub fn workflow_state_event(state: &WorkflowState) -> serde_json::Value {
    let progress = state.progress();
    let steps: Vec<serde_json::Value> = state
        .steps()
        .iter()
        .zip(state.done_flags().iter())
        .map(|(step, &done)| {
            serde_json::json!({
                "id": step.id,
                "label": step.label,
                "phase": step.phase,
                "done": done,
            })
        })
        .collect();

    let mut event = serde_json::json!({
        "type": "workflow_state",
        "steps": steps,
        "progress": {
            "done": progress.done,
            "total": progress.total,
            "percent": progress.percent,
        },
    });

    if let Some((num, title)) = state.active_issue() {
        event["activeIssue"] = serde_json::json!({
            "number": num,
            "title": title,
        });
    }

    event
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── WorkflowState tests ─────────────────────────────────────────────────

    #[test]
    fn test_default_state_has_16_steps() {
        let state = WorkflowState::default_bdd();
        assert_eq!(state.steps().len(), 16);
        assert!(state.done_flags().iter().all(|&d| !d));
        assert!(state.active_issue().is_none());
    }

    #[test]
    fn test_check_step() {
        let mut state = WorkflowState::default_bdd();
        state.check(1).unwrap();
        assert!(state.is_done(1).unwrap());
        assert!(!state.is_done(2).unwrap());
    }

    #[test]
    fn test_uncheck_step() {
        let mut state = WorkflowState::default_bdd();
        state.check(1).unwrap();
        state.uncheck(1).unwrap();
        assert!(!state.is_done(1).unwrap());
    }

    #[test]
    fn test_check_enforces_ordering() {
        let mut state = WorkflowState::default_bdd();
        let err = state.check(3).unwrap_err();
        assert!(err.to_string().contains("complete step 1 first"));
    }

    #[test]
    fn test_check_allows_next_step() {
        let mut state = WorkflowState::default_bdd();
        state.check(1).unwrap();
        state.check(2).unwrap();
        assert!(state.is_done(2).unwrap());
    }

    #[test]
    fn test_skip_bypasses_ordering() {
        let mut state = WorkflowState::default_bdd();
        state.skip(5).unwrap();
        assert!(state.is_done(5).unwrap());
        assert!(!state.is_done(4).unwrap());
    }

    #[test]
    fn test_reset_clears_all() {
        let mut state = WorkflowState::default_bdd();
        state.check(1).unwrap();
        state.set_issue(42, "My feature".into());
        state.reset();
        assert!(state.done_flags().iter().all(|&d| !d));
        assert!(state.active_issue().is_none());
    }

    #[test]
    fn test_set_issue() {
        let mut state = WorkflowState::default_bdd();
        state.set_issue(42, "My feature".into());
        let issue = state.active_issue().unwrap();
        assert_eq!(issue.0, 42);
        assert_eq!(issue.1, "My feature");
    }

    #[test]
    fn test_clear_issue() {
        let mut state = WorkflowState::default_bdd();
        state.set_issue(42, "My feature".into());
        state.clear_issue();
        assert!(state.active_issue().is_none());
    }

    #[test]
    fn test_progress() {
        let mut state = WorkflowState::default_bdd();
        state.check(1).unwrap();
        state.check(2).unwrap();
        let progress = state.progress();
        assert_eq!(progress.done, 2);
        assert_eq!(progress.total, 16);
        assert_eq!(progress.percent, 12);
    }

    #[test]
    fn test_check_out_of_range() {
        let mut state = WorkflowState::default_bdd();
        let err = state.check(0).unwrap_err();
        assert!(err.to_string().contains("invalid step"));
        let err = state.check(17).unwrap_err();
        assert!(err.to_string().contains("invalid step"));
    }

    #[test]
    fn test_uncheck_out_of_range() {
        let mut state = WorkflowState::default_bdd();
        let err = state.uncheck(0).unwrap_err();
        assert!(err.to_string().contains("invalid step"));
    }

    // ─── WorkflowConfig tests ────────────────────────────────────────────────

    #[test]
    fn test_default_config() {
        let config = WorkflowConfig::default();
        assert!(config.enabled);
        assert_eq!(config.steps.len(), 16);
        assert_eq!(config.steps[0].id, 1);
        assert_eq!(config.steps[0].label, "Update Scenarios / Add new features");
        assert_eq!(config.steps[0].phase, "red");
        assert_eq!(config.steps[15].id, 16);
        assert_eq!(config.steps[15].label, "Move to local master and pull");
        assert_eq!(config.steps[15].phase, "ci_cd");
    }

    #[test]
    fn test_config_disabled() {
        let config = WorkflowConfig {
            enabled: false,
            steps: default_steps(),
        };
        assert!(!config.enabled);
    }

    #[test]
    fn test_config_deserialize() {
        let json = r#"{"enabled":true,"steps":[{"id":1,"label":"Test","phase":"red"}]}"#;
        let config: WorkflowConfig = serde_json::from_str(json).unwrap();
        assert!(config.enabled);
        assert_eq!(config.steps.len(), 1);
    }

    #[test]
    fn test_config_deserialize_empty() {
        let json = r#"{}"#;
        let config: WorkflowConfig = serde_json::from_str(json).unwrap();
        assert!(config.enabled);
        assert_eq!(config.steps.len(), 16);
    }

    // ─── System prompt snippet tests ─────────────────────────────────────────

    #[test]
    fn test_system_prompt_snippet_with_checked_step() {
        let mut state = WorkflowState::default_bdd();
        state.check(1).unwrap();
        let snippet = state.system_prompt_snippet();
        assert!(snippet.contains("1/16"));
        assert!(snippet.contains("CURRENT STEP"));
    }

    #[test]
    fn test_system_prompt_snippet_with_active_issue() {
        let mut state = WorkflowState::default_bdd();
        state.set_issue(42, "My feature".into());
        let snippet = state.system_prompt_snippet();
        assert!(snippet.contains("#42"));
        assert!(snippet.contains("My feature"));
    }

    #[test]
    fn test_system_prompt_snippet_no_issue() {
        let state = WorkflowState::default_bdd();
        let snippet = state.system_prompt_snippet();
        assert!(snippet.contains("(not set)"));
    }

    // ─── UDS event tests ─────────────────────────────────────────────────────

    #[test]
    fn test_workflow_state_event() {
        let mut state = WorkflowState::default_bdd();
        state.check(1).unwrap();
        state.set_issue(42, "My feature".into());
        let event = workflow_state_event(&state);
        assert_eq!(event["type"], "workflow_state");
        assert!(event["steps"].is_array());
        assert_eq!(event["steps"].as_array().unwrap().len(), 16);
        assert_eq!(event["steps"][0]["done"], true);
        assert_eq!(event["steps"][1]["done"], false);
        assert_eq!(event["progress"]["done"], 1);
        assert_eq!(event["progress"]["total"], 16);
        assert_eq!(event["progress"]["percent"], 6);
        assert_eq!(event["activeIssue"]["number"], 42);
        assert_eq!(event["activeIssue"]["title"], "My feature");
    }

    #[test]
    fn test_workflow_state_event_no_issue() {
        let state = WorkflowState::default_bdd();
        let event = workflow_state_event(&state);
        assert!(event.get("activeIssue").is_none());
    }

    // ─── WorkflowError tests ─────────────────────────────────────────────────

    #[test]
    fn test_workflow_error_display() {
        let err = WorkflowError::InvalidStep("invalid step 0".into());
        assert_eq!(err.to_string(), "invalid step 0");
        let err = WorkflowError::OrderingViolation("complete step 1 first".into());
        assert_eq!(err.to_string(), "complete step 1 first");
    }
}
