//! #2403 review M5: the prompt a one-shot `-m` run sends is saved marked a
//! prompt, so a resumed watermark context pins it.

use super::super::AgentOutput;
use super::super::integration_tests::test_flags;
use super::super::run_session::run_agent_session;
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::application::providers::ports::{ChatRequest, LlmProvider};
use crate::domain::error::DomainError;
use crate::domain::message::LlmResponse;
use crate::interface::cli::run_end_fleet::RunHandles;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

#[derive(Debug)]
struct Answers;

impl LlmProvider for Answers {
    fn name(&self) -> &str {
        "answers"
    }

    fn chat(
        &self,
        _request: ChatRequest<'_>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + '_>> {
        Box::pin(async {
            Ok(LlmResponse {
                content: Some("the answer".into()),
                tool_calls: vec![],
                usage: None,
                stop_reason: None,
                thinking_blocks: vec![],
            })
        })
    }
}

fn agent() -> AgentLoopImpl {
    AgentLoopImpl::new(AgentLoopConfig {
        provider: Arc::new(Answers),
        tool_registry: Box::new(crate::infrastructure::tools::registry::ToolRegistryImpl::new()),
        model: "test-model".to_string(),
        max_tokens: 100,
        temperature: 0.0,
        retention: None,
        session_key: String::new(),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        large_result_collapse: crate::domain::large_result_collapse::LargeResultCollapse::DISABLED,
        model_context_window: None,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    })
}

/// Every session file under `dir`, read whole.
fn saved_sessions(dir: &std::path::Path) -> Vec<String> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        match path.is_dir() {
            true => found.extend(saved_sessions(&path)),
            false if path.extension().is_some_and(|ext| ext == "json") => {
                found.extend(std::fs::read_to_string(&path).ok());
            }
            false => {}
        }
    }
    found
}

#[test]
fn a_one_shot_prompt_is_saved_marked_a_prompt() {
    let tmp = tempfile::TempDir::new().unwrap();
    let flags = test_flags(Some("the one-shot prompt"), Some("marked"), None);
    let (mut stdout, mut stderr) = (String::new(), String::new());
    let mut out = AgentOutput {
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    let code = run_agent_session(
        tmp.path(),
        crate::composition::sessions::build_session_handles,
        agent(),
        RunHandles::without_children(&crate::composition::sessions::build_retention_handles(
            tmp.path(),
        )),
        &flags,
        &mut out,
    );
    assert_eq!(code, 0, "stderr: {stderr}");
    let saved = saved_sessions(&tmp.path().join("sessions"));
    let session = saved
        .iter()
        .find(|text| text.contains("the one-shot prompt"))
        .unwrap_or_else(|| panic!("the session was saved: {saved:?}"));
    let line = session
        .lines()
        .find(|line| line.contains("the one-shot prompt"))
        .unwrap();
    assert!(line.contains(r#""user_kind":"prompt""#), "{line}");
}
