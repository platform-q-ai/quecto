//! Contract for the `ReloadRuntime` port (#1849): the agent loop routes
//! through the swapped provider from the next request on, and replaces its
//! persisted tool-policy baseline with the entries it is handed — applying
//! the known ones, reporting the unknown ids, and dropping the live-only
//! overlays layered over the previous baseline.
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use quecto::application::agent_turn::ports::AgentLoop;
use quecto::application::catalogue::ports::ReloadRuntime;
use quecto::application::providers::ports::{ChatRequest, LlmProvider};
use quecto::domain::error::DomainError;
use quecto::domain::message::{LlmResponse, Message};
use quecto::domain::tool::{ToolPolicyApplyMode, ToolPolicyMutation};
use quecto::domain::tool_descriptor::ProfileAvailabilityScope;

use super::switch_runtime_fixture::runtime;

#[derive(Debug)]
struct Swapped;

impl LlmProvider for Swapped {
    fn name(&self) -> &str {
        "swapped"
    }
    fn chat<'a>(
        &'a self,
        _request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        Box::pin(async {
            Ok(LlmResponse {
                content: Some("from the swapped provider".into()),
                tool_calls: vec![],
                usage: None,
                stop_reason: None,
                thinking_blocks: vec![],
            })
        })
    }
}

#[tokio::test]
async fn the_swapped_provider_answers_the_next_request() {
    let mut rt = runtime("cli:reload-contract");
    ReloadRuntime::swap_provider(&mut rt.agent, Arc::new(Swapped));
    let mut messages = vec![Message::user("hello")];
    let result = rt.agent.process(&mut messages).await.unwrap();
    assert_eq!(result.response, "from the swapped provider");
}

#[test]
fn the_persisted_baseline_replaces_the_previous_one_and_clears_live_overlays() {
    let mut rt = runtime("cli:reload-contract");
    let entry = |rt: &quecto::application::agent_loop::AgentLoopImpl| {
        rt.tool_catalogue_entries()
            .into_iter()
            .find(|entry| entry.name == "key_observer")
            .expect("the fixture's tool is catalogued")
    };
    let stable_id = entry(&rt.agent).stable_id.into_owned();
    assert!(entry(&rt.agent).effective_enabled);

    // A baseline that disables the tool, plus an entry no tool matches.
    let mut entries = HashMap::new();
    entries.insert(stable_id.clone(), ProfileAvailabilityScope::None);
    entries.insert("native:ghost".to_string(), ProfileAvailabilityScope::None);
    let unknown = ReloadRuntime::apply_persisted_tool_policy(&mut rt.agent, &entries);
    assert_eq!(unknown, vec!["native:ghost".to_string()]);
    let disabled = entry(&rt.agent);
    assert_eq!(disabled.profile_scope, Some(ProfileAvailabilityScope::None));
    assert!(!disabled.effective_enabled);

    // The next baseline replaces it wholesale: an entry that is gone from
    // the config no longer applies.
    let unknown = ReloadRuntime::apply_persisted_tool_policy(&mut rt.agent, &HashMap::new());
    assert!(unknown.is_empty());
    let restored = entry(&rt.agent);
    assert_eq!(restored.profile_scope, None);
    assert!(restored.effective_enabled, "{restored:?}");

    // A live-only overlay (a session `set_tool_policy` without persist) is
    // dropped by the reload: the durable config is the baseline again.
    rt.agent.request_tool_policy_mutation(
        &[ToolPolicyMutation::set_scope(
            "key_observer",
            ProfileAvailabilityScope::Child,
            "live only",
        )],
        ToolPolicyApplyMode::ImmediateIfIdle,
    );
    assert!(entry(&rt.agent).session_enabled.is_some());
    ReloadRuntime::apply_persisted_tool_policy(&mut rt.agent, &HashMap::new());
    assert!(
        entry(&rt.agent).session_enabled.is_none(),
        "the live overlay was cleared by the reload"
    );
}
