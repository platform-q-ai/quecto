use std::sync::Arc;

use crate::domain::catalogue::{
    AuthIdentity, Availability, ModelCapabilities, ModelCost, ModelDescriptor, ModelRef,
    TransportKind,
};
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

fn descriptor(provider: &str, model: &str, auth: AuthIdentity) -> ModelDescriptor {
    ModelDescriptor {
        reference: ModelRef::parse(provider, model).unwrap(),
        display_name: Some(model.to_string()),
        transport: TransportKind::OpenAiCompletions,
        auth,
        base_url: None,
        auth_header: true,
        allow_remote_http: false,
        capabilities: ModelCapabilities {
            input: vec![],
            context_window: 0,
            max_tokens: 0,
            context_window_explicit: false,
            max_tokens_explicit: false,
            reasoning: false,
            cost: ModelCost::default(),
        },
        configured: true,
        availability: Availability::Runnable,
    }
}

#[test]
fn list_models_data_serializes_current_router_snapshot_not_models_json() {
    let runtime: Arc<dyn LlmProvider> = Arc::new(ProviderRouter::with_model_descriptors(
        vec![provider("current"), provider("other")],
        vec![
            descriptor("current", "current", AuthIdentity::ApiKey),
            descriptor("other", "other", AuthIdentity::ApiKey),
        ],
    ));

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
    let runtime: Arc<dyn LlmProvider> = Arc::new(ProviderRouter::with_model_descriptors(
        vec![provider("solo")],
        vec![descriptor("solo", "solo", AuthIdentity::ApiKey)],
    ));

    let data = list_models_data(&runtime);

    assert!(data.get("error").is_none(), "unexpected error: {data}");
    let models = data["models"].as_array().unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0]["provider"], "solo");
    assert_eq!(models[0]["model"], "solo/solo");
    assert_eq!(models[0]["configured"], true);
}
