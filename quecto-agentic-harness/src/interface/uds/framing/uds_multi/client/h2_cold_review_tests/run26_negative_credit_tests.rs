//! Real composed provider-view signed-credit regression.
use super::*;
use crate::application::agent_turn::ports::AgentLoop;
use crate::domain::agents::value_objects::turn_outcome::DECLARE_GUIDANCE;
use crate::domain::conversation::value_objects::message::Message;
use crate::domain::tool_policy::value_objects::tool::{ToolPolicyApplyMode, ToolPolicyRequest};
use crate::domain::tool_policy::value_objects::tool_descriptor::ProfileAvailabilityScope;

#[tokio::test]
async fn run26_disabled_guidance_credit_does_not_cut_fitting_history() {
    use crate::interface::cli::controllers::active_runtime_2708_tests::isolated;
    if isolated(
        concat!(
            module_path!(),
            "::run26_disabled_guidance_credit_does_not_cut_fitting_history"
        ),
        &["unset", "1"],
    ) {
        check_fitting_history(false).await;
    }
}

#[tokio::test]
async fn run26_normalized_guidance_control_preserves_fitting_history() {
    use crate::interface::cli::controllers::active_runtime_2708_tests::isolated;
    if isolated(
        concat!(
            module_path!(),
            "::run26_normalized_guidance_control_preserves_fitting_history"
        ),
        &["unset", "1"],
    ) {
        check_fitting_history(true).await;
    }
}

async fn check_fitting_history(normalized: bool) {
    use crate::interface::cli::controllers::active_runtime_2708_tests::compose;
    let composed = compose(true);
    let canonical = crate::interface::cli::controllers::shared::build_agent_system_prompt(
        Some("AGENTS negative-credit owner"),
        None,
        false,
        &composed.snippets,
    );
    assert!(canonical.contains(DECLARE_GUIDANCE));
    let provider = Arc::new(NegativeProvider::default());
    let mut messages = vec![Message::system(canonical.clone())];
    for turn in 1..=6 {
        let mut m = Message::assistant("historical context ".repeat(120), vec![]);
        m.turn = Some(turn);
        m.spill_id = Some(format!("turn{turn}:msg:assistant"));
        messages.push(m);
    }
    messages.push(Message::user("Legitimate fitting request"));
    let initial = messages
        .iter()
        .map(Message::estimated_tokens)
        .sum::<usize>();
    let mut projected = messages[0].clone();
    projected.content = canonical.replace(DECLARE_GUIDANCE, "");
    projected.invalidate_token_cache();
    let credit = messages[0].estimated_tokens() - projected.estimated_tokens();
    assert!(credit > 4);
    let ceiling = initial - credit / 2;
    // Independent real composition updates the extensions byte-length header.
    // Raw replacement is retained only as the nonnormative stale-header control.
    let effective = Message::system(
        crate::interface::cli::controllers::shared::build_agent_system_prompt(
            Some("AGENTS negative-credit owner"),
            None,
            false,
            &composed.snippets.replace(DECLARE_GUIDANCE, ""),
        ),
    );
    let expected = if normalized {
        initial - credit
    } else {
        initial - messages[0].estimated_tokens() + effective.estimated_tokens()
    };
    assert_eq!(initial, 9335);
    assert_eq!(ceiling, 9309);
    assert!(expected < ceiling);
    // The stale-header normalized control must still retain eight messages.
    if normalized {
        messages[0] = projected;
    }
    let original_ids: Vec<_> = messages.iter().map(Message::id).collect();
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: provider.clone(),
        tool_registry: Box::new(composed.registry),
        model: "stub".into(),
        max_tokens: 128,
        temperature: 0.0,
        retention: None,
        session_key: KEY.into(),
        max_context_tokens: ceiling,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_marks: crate::domain::conversation::services::watermark::Watermark::new(
            ceiling,
            ceiling * 8 / 10,
        )
        .unwrap(),
        model_context_window: None,
        tool_profile_context:
            crate::domain::tool_policy::value_objects::tool::ToolProfileContext::Parent,
    });
    agent
        .request_tool_policy(
            ToolPolicyRequest::replace(vec![], ProfileAvailabilityScope::None),
            ToolPolicyApplyMode::ImmediateIfIdle,
        )
        .unwrap();
    assert!(!agent.parent_outcome_candidate().unwrap().enabled());
    agent.bind_canonical_prompt(&messages, &canonical);
    {
        let run = agent.process(&mut messages);
        tokio::pin!(run);
        tokio::select! { _=provider.entered.notified()=>{}, _=&mut run=>panic!("unexpected completion"), _=tokio::time::sleep(STEP)=>panic!("no provider request") }
    }
    let (actual, sent, output) = provider.seen.lock().unwrap().clone().unwrap();
    println!(
        "RUN26 negative-credit canonical={initial} ceiling={ceiling} credit={credit} expected_effective={} actual={actual} before={} after={}",
        expected,
        original_ids.len(),
        sent.len()
    );
    assert!(
        expected < ceiling,
        "legitimate effective history fits below high/ceiling"
    );
    assert!(sent.iter().all(|m| !m.content.contains(DECLARE_GUIDANCE)));
    assert_eq!(output, 128);
    assert_eq!(
        sent.len(),
        8,
        "provider-fitting history must retain all eight messages"
    );
    assert_eq!(
        messages.iter().map(Message::id).collect::<Vec<_>>(),
        original_ids,
        "provider-fitting history must not be cut due to disabled canonical guidance"
    );
    assert_eq!(
        actual, expected,
        "provider receives exactly the independently composed fitting view"
    );
    if normalized {
        assert_eq!(actual, 9283, "nonnormative stale-header control");
    }
}

#[derive(Debug, Default)]
struct NegativeProvider {
    entered: Notify,
    seen: Mutex<Option<(usize, Vec<Message>, u32)>>,
}
impl LlmProvider for NegativeProvider {
    fn name(&self) -> &str {
        "run26-negative"
    }
    fn route_order(&self) -> Vec<String> {
        vec![self.name().into()]
    }
    fn chat<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        assert!(
            request.tools.is_empty(),
            "Replace None really disables all schema tokens"
        );
        let estimate = request
            .messages
            .iter()
            .map(Message::estimated_tokens)
            .sum::<usize>();
        *self.seen.lock().unwrap() =
            Some((estimate, request.messages.to_vec(), request.max_tokens));
        Box::pin(async move {
            self.entered.notify_one();
            std::future::pending().await
        })
    }
}
