use std::sync::Arc;

use crate::domain::message::LlmResponse;
use crate::domain::provider::{ChatRequest, LlmProvider};
use crate::infrastructure::providers::router::ProviderRouter;

use super::list_models_data;

#[derive(Debug)]
struct NamedProvider(String);

impl LlmProvider for NamedProvider {
    fn name(&self) -> &str {
        &self.0
    }

    fn chat<'a>(
        &'a self,
        _request: ChatRequest<'a>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<LlmResponse, crate::domain::error::DomainError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async { unreachable!("list_models must only inspect runtime provider names") })
    }
}

fn provider(name: &str) -> Arc<dyn LlmProvider> {
    Arc::new(NamedProvider(name.to_string()))
}

#[test]
fn list_models_data_serializes_current_router_snapshot_not_models_json() {
    let runtime: Arc<dyn LlmProvider> = Arc::new(ProviderRouter::new(vec![
        provider("current"),
        provider("other"),
    ]));

    let data = list_models_data(&runtime);

    assert!(data.get("error").is_none(), "unexpected error: {data}");
    let models = data["models"].as_array().unwrap();
    assert_eq!(models.len(), 2);
    assert!(
        models
            .iter()
            .any(|model| model["model"] == "current/current")
    );
    assert!(models.iter().any(|model| model["model"] == "other/other"));
}

#[test]
fn list_models_data_reports_current_single_provider_snapshot() {
    let runtime = provider("solo");

    let data = list_models_data(&runtime);

    assert!(data.get("error").is_none(), "unexpected error: {data}");
    let models = data["models"].as_array().unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0]["provider"], "solo");
    assert_eq!(models[0]["model"], "solo/solo");
    assert_eq!(models[0]["configured"], true);
}
