//! Workflow V2 domain model.
//!
//! Workflow is a native in-process subsystem owned by the UDS session runtime.
//! It exposes a template library, a single active workflow run, prompt snippets,
//! guard evaluation, nudges, and persistable run state.

use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

const MAX_TEMPLATE_COUNT: usize = 32;
const MAX_STEPS_PER_TEMPLATE: usize = 100;
const MAX_ISSUE_TITLE_LEN: usize = 500;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowTemplateStep {
    pub key: String,
    pub label: String,
    pub phase: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guidance: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowGuardRule {
    pub commands: Vec<String>,
    pub before_step_key: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowTemplate {
    pub id: String,
    pub label: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when_to_use: Option<String>,
    pub steps: Vec<WorkflowTemplateStep>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guards: Vec<WorkflowGuardRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowRunPersisted {
    pub template_id: Option<String>,
    pub done: Vec<bool>,
    pub active_issue: Option<(u32, String)>,
}

/// Maximum serialized size of a by-value [`WorkflowSpec`] (256 KiB). Bounds the
/// inline template a parent may hand a child, on both the write and read sides,
/// so a malformed/hostile spec cannot exhaust memory at each recursion level.
pub const MAX_WORKFLOW_SPEC_BYTES: usize = 256 * 1024;

/// A by-value, binding workflow assignment handed to a spawned sub-agent.
///
/// Carries the **full template definition** (not an id reference), so a parent
/// can hand a child a sub-workflow without the child's config having to define
/// it. An assigned child runs exactly this template in `Active` mode and cannot
/// select another (binding). Unknown fields are ignored so the spec can carry
/// future additions (inputs, acceptance, budget) without breaking older agents.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowSpec {
    /// The template the assigned sub-agent must run.
    pub template: WorkflowTemplate,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowConfig {
    #[serde(default = "default_true")]
    pub auto_continue: bool,
    #[serde(default = "default_true")]
    pub completion_nudge: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub templates: Vec<WorkflowTemplate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowMode {
    SelectingTemplate,
    Active,
    Complete,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkflowRun {
    pub template_id: Option<String>,
    pub template_index: Option<usize>,
    pub done: Vec<bool>,
    pub active_issue: Option<(u32, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowTemplateSummary {
    pub id: String,
    pub label: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when_to_use: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowStepStatus {
    pub index: u32,
    pub key: String,
    pub label: String,
    pub phase: String,
    pub done: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guidance: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowProgress {
    pub done: u32,
    pub total: u32,
    pub percent: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkflowSnapshot {
    pub enabled: bool,
    pub guards_enabled: bool,
    pub mode: WorkflowMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_template: Option<WorkflowTemplateSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_issue: Option<(u32, String)>,
    pub progress: WorkflowProgress,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_step: Option<WorkflowStepStatus>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<WorkflowStepStatus>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available_templates: Vec<WorkflowTemplateSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkflowError {
    UnknownTemplate(String),
    InvalidStep(String),
    OrderingViolation(String),
    NoActiveTemplate(String),
    InvalidConfig(String),
    GuardBlocked(String),
}

impl std::fmt::Display for WorkflowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownTemplate(s)
            | Self::InvalidStep(s)
            | Self::OrderingViolation(s)
            | Self::NoActiveTemplate(s)
            | Self::InvalidConfig(s)
            | Self::GuardBlocked(s) => write!(f, "{}", s),
        }
    }
}
impl std::error::Error for WorkflowError {}

impl Default for WorkflowConfig {
    fn default() -> Self {
        Self {
            auto_continue: true,
            completion_nudge: true,
            selector_prompt: None,
            templates: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct WorkflowEngine {
    templates: Vec<WorkflowTemplate>,
    run: WorkflowRun,
    auto_continue: bool,
    completion_nudge: bool,
    guards_enabled: bool,
    selector_prompt: Option<String>,
    /// When true the engine is bound to a single assigned template (a by-value
    /// `--workflow-spec`): it cannot return to template selection (`reset` only
    /// clears step progress), `select_template` cannot switch templates, and the
    /// completion nudge does not tell the model to pick a new workflow.
    bound: bool,
}

mod engine;
pub use engine::*;

#[cfg(test)]
#[path = "workflow_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "workflow_comprehensive_tests.rs"]
mod comprehensive_tests;
