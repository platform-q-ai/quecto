//! #2212 agent-loop-level tests: every provider-reported prompt size
//! calibrates the next pruning pass of the same run, and a model switch
//! forgets the calibration.

use crate::application::agent_loop::AgentLoopImpl;
use crate::application::agent_loop::tests::{
    MockProvider, MockRegistry, MockTool, test_config, text_response, tool_call_response,
};
use crate::domain::context_calibration::EstimateScale;
use crate::domain::message::{LlmResponse, Message, UsageInfo};
use std::sync::Arc;

/// Six spilled prior turns of about 500 estimated tokens each.
fn history() -> Vec<Message> {
    (1..=6)
        .map(|turn| {
            let mut msg = Message::assistant("x".repeat(2_000), vec![]);
            msg.turn = Some(turn);
            msg.spill_id = Some(format!("turn{turn}:msg:assistant"));
            msg
        })
        .collect()
}

fn reporting(mut response: LlmResponse, context_tokens: Option<u32>) -> LlmResponse {
    response.usage = context_tokens.map(|tokens| UsageInfo {
        prompt_tokens: tokens,
        completion_tokens: 5,
        cache_read_tokens: None,
        cache_write_tokens: None,
        context_tokens: Some(tokens),
        cost: None,
    });
    response
}

/// A run whose first reply is a tool call reporting `reported` context
/// tokens; the budget holds the estimate with room to spare.
async fn run_with_first_report(reported: Option<u32>) -> (AgentLoopImpl, Vec<Message>) {
    let provider = Arc::new(MockProvider::new(vec![
        reporting(tool_call_response("lookup", "{}"), reported),
        reporting(text_response("done"), None),
    ]));
    let mut registry = MockRegistry::new();
    registry.register(Arc::new(MockTool::new("lookup", "ok")));
    let mut agent = AgentLoopImpl::new(crate::application::agent_loop::AgentLoopConfig {
        max_context_tokens: 4_000,
        ..test_config(provider, Box::new(registry))
    });
    let mut messages = history();
    messages.push(Message::user("go"));
    agent.run_loop(&mut messages).await.unwrap();
    (agent, messages)
}

fn stubbed(messages: &[Message]) -> usize {
    messages.iter().filter(|m| m.is_collapsed).count()
}

#[tokio::test]
async fn without_reported_usage_the_run_prunes_on_the_heuristic() {
    let (agent, messages) = run_with_first_report(None).await;
    assert_eq!(stubbed(&messages), 0, "3000 estimated tokens fit 4000");
    assert_eq!(
        agent.context_manager.estimate_scale(),
        EstimateScale::IDENTITY
    );
}

fn first_request_messages_estimate() -> usize {
    let mut messages = history();
    messages.push(Message::user("go"));
    crate::application::context_pruning::estimate_total_tokens(&messages)
}

/// The QA shape: the provider counts the transcript at several times the
/// estimate; the next request of the same run is pruned on that figure.
/// The final reply reports no usage, so the scale read after the run is
/// the one the first reply set (the tool definitions make the estimate a
/// little larger than the messages alone, hence the range).
#[tokio::test]
async fn a_reported_size_mid_run_prunes_the_next_request_of_the_same_run() {
    let reported = first_request_messages_estimate() * 3;
    let (agent, messages) = run_with_first_report(Some(reported as u32)).await;
    assert!(
        stubbed(&messages) > 0,
        "at 3x the estimate the transcript is over the 4000 budget"
    );
    let permille = agent.context_manager.estimate_scale().permille();
    assert!((2_900..=3_000).contains(&permille), "{permille}");
}

#[tokio::test]
async fn a_tenfold_report_is_capped_at_four_times_the_estimate() {
    let reported = first_request_messages_estimate() * 10;
    let (agent, messages) = run_with_first_report(Some(reported as u32)).await;
    assert!(stubbed(&messages) > 0);
    assert_eq!(
        agent.context_manager.estimate_scale().permille(),
        EstimateScale::MAX_PERMILLE
    );
}

/// Review probe A (#2212): the final reply reports no usage, so the run's
/// last usage is the first reply's. Pairing it with the final request's
/// (post-prune) estimate at the end of the run inflated the scale (2x read
/// as 4x) and carried it into the next run.
#[tokio::test]
async fn the_end_of_a_run_never_pairs_an_earlier_report_with_a_later_estimate() {
    let reported = first_request_messages_estimate() * 2;
    let (agent, messages) = run_with_first_report(Some(reported as u32)).await;
    assert!(
        stubbed(&messages) > 0,
        "2x is over the budget: the run pruned"
    );
    let permille = agent.context_manager.estimate_scale().permille();
    assert!(
        permille <= 2_000,
        "the one real observation is 2x, got {permille}"
    );
}

/// Review probe B (#2212): the output limit raised after a cut-off leaves
/// the window room beside the prompt at its calibrated size, not its raw
/// estimate. (The raised limit never drops below the normal one, so the
/// probe keeps the prompt where the normal limit still fits.)
#[test]
fn the_raised_output_limit_fits_beside_the_calibrated_prompt() {
    let provider = Arc::new(MockProvider::new(vec![]));
    let agent = AgentLoopImpl::new(crate::application::agent_loop::AgentLoopConfig {
        max_context_tokens: 200_000,
        max_tokens: 32_000,
        ..test_config(provider, Box::new(MockRegistry::new()))
    })
    .with_model_max_tokens(Some(128_000))
    .with_model_context_window(Some(200_000));
    // The provider counted 150k for a 75k estimate: the prompt is 150k.
    agent.observe_provider_context_gauge_for_test(150_000, 75_000);
    agent
        .output_boost
        .store(true, std::sync::atomic::Ordering::Relaxed);

    let limit = agent.request_max_tokens(75_000) as usize;

    assert!(limit >= 32_000, "never below the normal limit: {limit}");
    assert!(
        150_000 + limit <= 200_000,
        "prompt + max_tokens overflows the window: {limit}"
    );
}

/// A provider that answers nothing, under a chosen name.
#[derive(Debug)]
struct NamedProvider(&'static str);

impl crate::application::providers::ports::LlmProvider for NamedProvider {
    fn name(&self) -> &str {
        self.0
    }

    fn chat(
        &self,
        _request: crate::application::providers::ports::ChatRequest<'_>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<LlmResponse, crate::domain::error::DomainError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async { Ok(text_response("unused")) })
    }
}

/// A config reload may change the endpoint behind the same provider name
/// (a rebuilt router is always `router`), so it forgets the observed scale
/// and the provider figure, as a model switch does.
#[tokio::test]
async fn a_reload_to_another_provider_forgets_the_calibration() {
    let (mut agent, _) = crate::application::agent_loop::tests::make_agent(vec![], vec![]);
    agent.observe_provider_context_gauge_for_test(2_000, 1_000);

    agent.swap_provider(Arc::new(NamedProvider("other-provider")));

    assert_eq!(
        agent.context_manager.estimate_scale(),
        EstimateScale::IDENTITY
    );
    assert_eq!(
        agent.reconcile_context_gauge_for_test(1_100),
        1_100,
        "the old tokeniser's occupancy is not shown for the new provider"
    );
}

#[tokio::test]
async fn a_reload_under_the_same_provider_name_forgets_the_calibration() {
    let (mut agent, _) = crate::application::agent_loop::tests::make_agent(vec![], vec![]);
    agent.observe_provider_context_gauge_for_test(2_000, 1_000);

    agent.swap_provider(Arc::new(NamedProvider("mock")));

    assert_eq!(
        agent.context_manager.estimate_scale(),
        EstimateScale::IDENTITY
    );
    assert_eq!(agent.reconcile_context_gauge_for_test(1_100), 1_100);
}

/// #2212 review 2: the gauge recorded at the tool-iteration limit counts
/// the tool definitions, as every other estimate path does.
#[tokio::test]
async fn the_iteration_limit_estimate_counts_the_tool_definitions() {
    let provider = Arc::new(MockProvider::new(vec![tool_call_response("lookup", "{}")]));
    let mut registry = MockRegistry::new();
    registry.register(Arc::new(MockTool::new("lookup", "ok")));
    let mut agent =
        AgentLoopImpl::new(test_config(provider, Box::new(registry))).with_max_tool_iterations(1);
    let mut messages = vec![Message::user("go")];

    let result = agent.run_loop(&mut messages).await.unwrap();

    assert!(result.iteration_limit_reached);
    let tools = agent.tool_definition_tokens();
    assert!(tools > 0);
    assert_eq!(
        result.context_tokens,
        crate::application::context_pruning::estimate_total_tokens(&messages) + tools
    );
}

#[tokio::test]
async fn a_model_switch_forgets_the_observed_scale() {
    let (mut agent, _) = crate::application::agent_loop::tests::make_agent(vec![], vec![]);
    agent.observe_provider_context_gauge_for_test(2_000, 1_000);
    assert_eq!(agent.context_manager.estimate_scale().permille(), 2_000);

    crate::application::catalogue::ports::ModelRuntime::apply_model(
        &mut agent,
        "other/model".into(),
        crate::application::catalogue::dto::ModelLimits {
            max_output_tokens: None,
            context_window: None,
        },
    );

    assert_eq!(
        agent.context_manager.estimate_scale(),
        EstimateScale::IDENTITY
    );
    assert_eq!(
        agent.context_manager.pruning_ceiling_in_estimate_units(),
        agent.effective_max_context_tokens()
    );
    assert_eq!(
        agent.reconcile_context_gauge_for_test(1_100),
        1_100,
        "the old model's occupancy is not shown for the new one"
    );
}

/// Audit sink capturing the `budget_unmet` flag of every ContextPruned.
#[derive(Debug, Default)]
struct PrunedAudit {
    unmet: std::sync::Mutex<Vec<bool>>,
}

impl crate::application::audit::ports::AuditSink for PrunedAudit {
    fn emit(
        &self,
        _turn: u32,
        event: crate::domain::audit::AuditEvent,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<(), crate::domain::error::DomainError>>
                + Send
                + '_,
        >,
    > {
        if let crate::domain::audit::AuditEvent::ContextPruned { budget_unmet, .. } = event {
            self.unmet.lock().unwrap().push(budget_unmet);
        }
        Box::pin(async { Ok(()) })
    }
}

/// An agent with 40 tools, a model window of `window_tenths_of_tools`
/// tenths of the tools' estimate, and a provider scale of 2x observed.
fn tool_heavy_agent(
    window_tenths_of_tools: usize,
) -> (AgentLoopImpl, usize, usize, Arc<PrunedAudit>) {
    let mut registry = MockRegistry::new();
    for i in 0..40 {
        registry.register(Arc::new(MockTool::new(&format!("tool_{i}"), "ok")));
    }
    let tools: usize = registry
        .cached_definitions
        .iter()
        .map(crate::domain::tool::ToolDefinition::estimated_tokens)
        .sum();
    let window = tools * window_tenths_of_tools / 10;
    let audit = Arc::new(PrunedAudit::default());
    let agent = AgentLoopImpl::new(crate::application::agent_loop::AgentLoopConfig {
        audit_log: Some(audit.clone() as Arc<dyn crate::application::audit::ports::AuditSink>),
        ..test_config(Arc::new(MockProvider::new(vec![])), Box::new(registry))
    })
    .with_model_context_window(Some(window));
    agent.observe_provider_context_gauge_for_test(2_000, 1_000);
    (agent, tools, window, audit)
}

/// #2212 PR review: at a 2x scale the window in estimate units is half the
/// window; the quarter floor must not grant the messages more than the
/// tools leave of it.
#[tokio::test]
async fn tools_and_messages_fit_the_window_at_the_calibrated_size() {
    // Window 2.4x the tools: 1.2x in estimate units, so the tools leave
    // 0.2x of their size where the floor would grant 0.3x.
    let (agent, tools, window, audit) = tool_heavy_agent(24);
    // Eight small spilled turns: the two pinned ones fit what is left.
    let mut messages: Vec<Message> = (1..=8)
        .map(|turn| {
            let mut msg = Message::assistant("x".repeat(160), vec![]);
            msg.turn = Some(turn);
            msg.spill_id = Some(format!("turn{turn}:msg:assistant"));
            msg
        })
        .collect();
    messages.push(Message::user("go"));

    agent.apply_context_pruning(&mut messages, 1, false).await;

    let total = crate::application::context_pruning::estimate_total_tokens(&messages) + tools;
    let scale = agent.context_manager.estimate_scale();
    assert!(
        scale.calibrated(total) <= window,
        "{} calibrated tokens in a {window}-token window",
        scale.calibrated(total)
    );
    assert_eq!(*audit.unmet.lock().unwrap(), vec![false]);
}

#[tokio::test]
async fn tools_that_fill_the_window_are_reported_over_budget() {
    // Window 1.5x the tools: 0.75x in estimate units, under the tools alone.
    let (agent, _, _, audit) = tool_heavy_agent(15);
    let mut messages = vec![Message::user("go")];

    agent.apply_context_pruning(&mut messages, 1, false).await;

    assert_eq!(*audit.unmet.lock().unwrap(), vec![true]);
    assert_eq!(messages.len(), 1, "the in-flight prompt is kept");
}

/// #2212 PR review 2: the reviewer's shape without a model window (a 100k
/// budget, 2x, 45k of tools): the quarter floor keeps a minimum
/// conversation beside the tools although that passes the configured
/// (soft) budget, and the pass reports the budget unmet.
#[tokio::test]
async fn the_quarter_floor_over_the_calibrated_budget_is_reported_unmet() {
    let mut registry = MockRegistry::new();
    for i in 0..40 {
        registry.register(Arc::new(MockTool::new(&format!("tool_{i}"), "ok")));
    }
    let tools: usize = registry
        .cached_definitions
        .iter()
        .map(crate::domain::tool::ToolDefinition::estimated_tokens)
        .sum();
    let audit = Arc::new(PrunedAudit::default());
    let agent = AgentLoopImpl::new(crate::application::agent_loop::AgentLoopConfig {
        // 100k against 45k of tools, as the reviewer's example.
        max_context_tokens: tools * 100 / 45,
        audit_log: Some(audit.clone() as Arc<dyn crate::application::audit::ports::AuditSink>),
        ..test_config(Arc::new(MockProvider::new(vec![])), Box::new(registry))
    });
    agent.observe_provider_context_gauge_for_test(2_000, 1_000);
    let effective = agent.context_manager.pruning_ceiling_in_estimate_units();
    assert!(effective - tools < effective / 4, "the floor applies");
    let mut messages = vec![Message::user("go")];

    agent.apply_context_pruning(&mut messages, 1, false).await;

    assert_eq!(messages.len(), 1, "the floor keeps the conversation");
    assert_eq!(*audit.unmet.lock().unwrap(), vec![true]);
}
