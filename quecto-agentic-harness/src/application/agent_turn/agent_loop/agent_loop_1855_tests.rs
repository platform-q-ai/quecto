//! #1855, #2328: the turn costs usage at the active model's catalogue
//! price, the one price authority, whatever the provider reported; a model
//! the catalogue prices nothing for is uncosted.
use crate::application::agent_turn::agent_loop::tests::{
    MockProvider, MockRegistry, test_config, text_response,
};
use crate::application::agent_turn::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::application::catalogue::dto::ModelLimits;
use crate::domain::conversation::value_objects::message::{CostInfo, Message};
use crate::domain::inference::value_objects::model_price::ModelPrice;
use std::sync::Arc;

/// $2 in, $10 out (GPT-6.1 Sol, Sonnet 5).
const PRICE: ModelPrice = ModelPrice {
    input: 2_000_000,
    output: 10_000_000,
    cache_read: 100_000,
    cache_write: 2_500_000,
};

fn agent(price: Option<ModelPrice>) -> AgentLoopImpl {
    // text_response reports 10 prompt and 20 completion tokens.
    let mut reply = text_response("done");
    // A provider's own figure is never the authority: it is replaced.
    reply.usage.as_mut().unwrap().cost = Some(CostInfo {
        input_cost_micro_usd: 999,
        output_cost_micro_usd: 999,
        cache_read_cost_micro_usd: 0,
        cache_write_cost_micro_usd: 0,
        total_cost_micro_usd: 1998,
    });
    let provider = Arc::new(MockProvider::new(vec![reply]));
    AgentLoopImpl::new(AgentLoopConfig {
        ..test_config(provider, Box::new(MockRegistry::new()))
    })
    .with_model_limits(ModelLimits {
        price,
        ..ModelLimits::default()
    })
}

#[tokio::test]
async fn usage_is_costed_at_the_active_models_catalogue_price() {
    let mut agent = agent(Some(PRICE));
    let mut messages = vec![Message::user("hi")];
    let result = agent.run_loop(&mut messages).await.unwrap();
    // 10 * $2/M + 20 * $10/M = 20 + 200 micro-USD.
    assert_eq!(result.cost_micro_usd, 220);
    assert_eq!(agent.take_unreported_usage().cost_micro_usd, 220);
    let observed = agent.take_request_diagnostics();
    let costs: Vec<_> = observed
        .recent
        .iter()
        .map(|record| record.estimated_cost_micro_usd)
        .collect();
    assert_eq!(costs, [Some(220)], "the request record carries the cost");
}

#[tokio::test]
async fn a_model_the_catalogue_prices_nothing_for_is_uncosted() {
    let mut agent = agent(None);
    let mut messages = vec![Message::user("hi")];
    let result = agent.run_loop(&mut messages).await.unwrap();
    assert_eq!(result.cost_micro_usd, 0, "the provider's figure is dropped");
    assert_eq!(agent.take_unreported_usage().cost_micro_usd, 0);
}
