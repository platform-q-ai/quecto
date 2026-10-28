//! Provider-bound accounting uses the effective prompt, without persisting its view.
use super::*;
use crate::application::agent_turn::ports::AgentLoop;
use crate::application::tools::ports::ToolCatalog;
use crate::domain::agents::value_objects::agent::AgentProgressEvent;
use crate::domain::agents::value_objects::turn_outcome::DECLARE_GUIDANCE;
use crate::domain::conversation::value_objects::message::{Message, Role};
use crate::domain::tool_policy::value_objects::tool::{
    ToolPolicyApplyMode, ToolPolicyMutation, ToolPolicyRequest,
};
use crate::domain::tool_policy::value_objects::tool_descriptor::ProfileAvailabilityScope;
use crate::infrastructure::config::{Config, ToolPolicyEntryConfig};

#[derive(Debug, Default)]
struct BudgetProvider {
    entered: Notify,
    seen: Mutex<Option<(usize, Vec<Message>, u32)>>,
}
impl LlmProvider for BudgetProvider {
    fn name(&self) -> &str {
        "run24-budget"
    }
    fn route_order(&self) -> Vec<String> {
        vec![self.name().into()]
    }
    fn chat<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        let estimate = request
            .messages
            .iter()
            .map(Message::estimated_tokens)
            .sum::<usize>()
            + request
                .tools
                .iter()
                .map(|tool| tool.estimated_tokens())
                .sum::<usize>();
        *self.seen.lock().unwrap() =
            Some((estimate, request.messages.to_vec(), request.max_tokens));
        Box::pin(async move {
            self.entered.notify_one();
            std::future::pending().await
        })
    }
}

#[derive(Clone, Copy)]
enum Head {
    Composed,
    Absent,
    Manifest,
}

async fn measured_request(head: Head) {
    use crate::interface::cli::controllers::active_runtime_2708_tests::{
        compose, compose_with_config,
    };
    let active = compose(true);
    let stable = active
        .registry
        .catalogue_entries()
        .into_iter()
        .find(|e| e.name == "declare_turn_outcome")
        .unwrap()
        .stable_id
        .into_owned();
    let tools = active
        .registry
        .definitions()
        .iter()
        .map(|tool| tool.estimated_tokens())
        .sum::<usize>();
    drop(active);
    let mut config = Config::default();
    config.tools.policy.entries.insert(
        stable,
        ToolPolicyEntryConfig {
            scope: ProfileAvailabilityScope::None,
        },
    );
    let composed = compose_with_config(true, |_| {}, config);
    let runtime = composed
        .registry
        .parent_outcome_candidate()
        .expect("authorized startup-withheld candidate");
    assert!(
        !composed.snippets.contains(DECLARE_GUIDANCE),
        "real startup withholding"
    );
    let canonical = crate::interface::cli::controllers::shared::build_agent_system_prompt(
        Some("budget AGENTS owner"),
        None,
        false,
        &composed.snippets,
    );
    let mut messages = Vec::new();
    if matches!(head, Head::Composed | Head::Manifest) {
        let mut system = Message::system(canonical.clone());
        system.is_manifest = matches!(head, Head::Manifest);
        messages.push(system);
    }
    for turn in 1..=6 {
        let mut message = Message::assistant("historical context ".repeat(120), vec![]);
        message.turn = Some(turn);
        message.spill_id = Some(format!("turn{turn}:msg:assistant"));
        messages.push(message);
    }
    messages.push(Message::user("Budgeted request"));
    let provider = Arc::new(BudgetProvider::default());
    let gauges = Arc::new(Mutex::new(Vec::new()));
    let capture = gauges.clone();
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: provider.clone(),
        tool_registry: Box::new(composed.registry),
        model: "stub".into(),
        max_tokens: 128,
        temperature: 0.0,
        retention: None,
        session_key: KEY.into(),
        max_context_tokens: 190_000,
        progress_callback: Some(Arc::new(move |event| {
            if let AgentProgressEvent::Thinking { context_tokens, .. } = event {
                capture.lock().unwrap().push(context_tokens);
            }
        })),
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_marks: Default::default(),
        model_context_window: None,
        tool_profile_context:
            crate::domain::tool_policy::value_objects::tool::ToolProfileContext::Parent,
    });
    agent
        .request_tool_policy(
            ToolPolicyRequest::patch(vec![ToolPolicyMutation::enable(
                "declare_turn_outcome",
                "restore startup policy",
            )]),
            ToolPolicyApplyMode::ImmediateIfIdle,
        )
        .unwrap();
    assert!(agent.is_tool_model_visible("declare_turn_outcome"));
    // Match the production outer-boundary caller after effective policy restore;
    // process() itself intentionally does not admit/rearm a transport episode.
    runtime.enter_outer_boundary();
    assert!(
        runtime.enabled(),
        "restore re-arms only the authorized candidate"
    );
    let canonical_estimate = messages
        .iter()
        .map(Message::estimated_tokens)
        .sum::<usize>()
        + tools;
    // Shared window: reserve exactly the requested reply. Canonical fits with
    // one token slack; the effective guidance must be accounted BEFORE pruning.
    let room = canonical_estimate + 1;
    agent = agent.with_model_context_window(Some(room + 128));
    if matches!(head, Head::Composed) {
        agent.bind_canonical_prompt(&messages, &canonical);
    }
    let original_head = messages
        .iter()
        .find(|m| m.role == Role::System)
        .map(|m| (m.id(), m.content.clone(), m.estimated_tokens()));
    {
        let run = agent.process(&mut messages);
        tokio::pin!(run);
        tokio::select! {
            _ = provider.entered.notified() => {},
            _ = &mut run => panic!("pending boundary unexpectedly returned"),
            _ = tokio::time::sleep(STEP) => panic!("provider boundary not reached"),
        }
    }
    let (actual, sent, output) = provider.seen.lock().unwrap().clone().unwrap();
    let reported = *gauges.lock().unwrap().first().unwrap();
    println!(
        "RUN24 budget canonical={canonical_estimate} room={room} provider={actual} gauge={reported} output={output} guidance_bytes={}",
        DECLARE_GUIDANCE.len()
    );
    assert_eq!(
        output, 128,
        "shared-window reply reserve is not stolen to hide prompt excess"
    );
    if let Some((id, content, tokens)) = original_head {
        let restored = messages.iter().find(|m| m.id() == id).unwrap();
        assert_eq!(
            restored.content, content,
            "cancellation restores canonical/manifest head"
        );
        assert_eq!(
            restored.estimated_tokens(),
            tokens,
            "canonical token cache restored"
        );
    }
    assert_eq!(
        sent.iter().any(|m| m.content.contains(DECLARE_GUIDANCE)),
        matches!(head, Head::Composed),
        "no synthetic head or manifest ownership"
    );
    assert!(
        actual <= room,
        "effective provider-bound estimate {actual} exceeds measured shared prompt room {room}"
    );
    assert_eq!(
        reported, actual,
        "Thinking/accounting must measure the actual provider-bound prompt"
    );
}

#[tokio::test]
async fn run24_effective_guidance_fits_shared_window_before_provider() {
    if isolated(
        concat!(
            module_path!(),
            "::run24_effective_guidance_fits_shared_window_before_provider"
        ),
        &["unset", "1"],
    ) {
        measured_request(Head::Composed).await;
    }
}

#[tokio::test]
async fn run24_head_free_and_manifest_near_bound_are_legitimate() {
    if isolated(
        concat!(
            module_path!(),
            "::run24_head_free_and_manifest_near_bound_are_legitimate"
        ),
        &["unset", "1"],
    ) {
        measured_request(Head::Absent).await;
        measured_request(Head::Manifest).await;
    }
}
