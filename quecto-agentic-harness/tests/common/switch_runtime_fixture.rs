//! The loop runtime the session-switch contracts drive (D7 #1976): a real
//! agent loop over a silent provider with one session-aware tool, a real
//! tracker, a real execution view and a real workflow engine.
#![allow(dead_code)]
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use quecto::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use quecto::application::providers::ports::{ChatRequest, LlmProvider};
use quecto::application::tools::ports::Tool;
use quecto::domain::error::DomainError;
use quecto::domain::message::LlmResponse;
use quecto::domain::tool::{ToolDefinition, ToolResult};
use quecto::domain::workflow::{
    WorkflowConfig, WorkflowEngine, WorkflowTemplate, WorkflowTemplateStep,
};
use quecto::infrastructure::tools::registry::ToolRegistryImpl;
use quecto::interface::cli::uds_execution_state::{ExecutionState, ExecutionStateHandle};
use quecto::interface::cli::uds_session::AgentSession;
use quecto::interface::shared::WorkflowStateHandle;

#[derive(Debug)]
struct SilentProvider;

impl LlmProvider for SilentProvider {
    fn name(&self) -> &str {
        "silent"
    }
    fn chat<'a>(
        &'a self,
        _request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        Box::pin(async {
            Ok(LlmResponse {
                content: Some(String::new()),
                tool_calls: vec![],
                usage: None,
                stop_reason: None,
                thinking_blocks: vec![],
            })
        })
    }
}

/// Records every session key it is told.
#[derive(Default)]
pub struct KeyObservingTool {
    pub seen: Mutex<Vec<String>>,
}

impl Tool for KeyObservingTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "key_observer".into(),
            description: "records the session key".into(),
            parameters_schema: r#"{"type":"object"}"#.into(),
        }
    }
    fn set_session_key(&self, session_key: String) {
        self.seen.lock().unwrap().push(session_key);
    }
    fn execute(
        &self,
        _arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        Box::pin(async {
            Ok(ToolResult {
                content: String::new(),
                is_error: false,
                image_blocks: vec![],
                delivery_metadata: None,
            })
        })
    }
}

pub struct Runtime {
    pub agent: AgentLoopImpl,
    pub session: AgentSession,
    pub execution: ExecutionStateHandle,
    pub workflow: WorkflowStateHandle,
    pub tool: Arc<KeyObservingTool>,
    /// The change-reasoning-effort use case over an empty published
    /// catalogue (#1848): the fixture's `stub` model has no effort control.
    pub effort: Arc<quecto::application::catalogue::use_cases::ChangeReasoningEffort>,
    _catalogue_dir: tempfile::TempDir,
}

pub fn runtime(session_key: &str) -> Runtime {
    let tool = Arc::new(KeyObservingTool::default());
    let mut registry = ToolRegistryImpl::new();
    registry.register(tool.clone());
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: Arc::new(SilentProvider),
        tool_registry: Box::new(registry),
        model: "stub".into(),
        max_tokens: 16,
        temperature: 0.0,
        retention: None,
        session_key: session_key.into(),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens: 10_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
        tool_profile_context: quecto::domain::tool::ToolProfileContext::Parent,
    });
    let workflow = WorkflowEngine::new(
        WorkflowConfig {
            templates: vec![WorkflowTemplate {
                id: "feature".into(),
                label: "Feature".into(),
                description: "contract".into(),
                when_to_use: None,
                steps: (1..=2)
                    .map(|i| WorkflowTemplateStep {
                        key: format!("s{i}"),
                        label: format!("Step {i}"),
                        phase: "contract".into(),
                        guidance: None,
                    })
                    .collect(),
                guards: vec![],
            }],
            ..WorkflowConfig::default()
        },
        false,
    )
    .unwrap();
    let catalogue_dir = tempfile::TempDir::new().expect("catalogue dir");
    let effort =
        quecto::composition::catalogue::build_catalogue_handles(catalogue_dir.path(), None).effort;
    Runtime {
        agent,
        session: AgentSession::new("stub".into()),
        execution: Arc::new(Mutex::new(ExecutionState::default())),
        workflow: Arc::new(Mutex::new(workflow)),
        tool,
        effort,
        _catalogue_dir: catalogue_dir,
    }
}
