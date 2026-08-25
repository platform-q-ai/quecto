use std::sync::Arc;

use crate::domain::catalogue::{
    AuthIdentity, Availability, ModelCapabilities, ModelCost, ModelDescriptor, ModelRef,
    ProviderId, TransportKind,
};
use crate::domain::message::LlmResponse;
use crate::domain::provider::{ChatRequest, LlmProvider};
use crate::infrastructure::providers::retry::{RetryConfig, RetryingProvider};
use crate::infrastructure::providers::router::ProviderRouter;

use super::list_models_data;

#[derive(Debug)]
struct NamedProvider {
    name: String,
    descriptors: Vec<ModelDescriptor>,
}

impl LlmProvider for NamedProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn model_descriptors(&self) -> Option<&[ModelDescriptor]> {
        Some(&self.descriptors)
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
    Arc::new(NamedProvider {
        name: name.to_string(),
        descriptors: Vec::new(),
    })
}

fn provider_with_descriptors(
    name: &str,
    descriptors: Vec<ModelDescriptor>,
) -> Arc<dyn LlmProvider> {
    Arc::new(NamedProvider {
        name: name.to_string(),
        descriptors,
    })
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

trait DescriptorTestExt {
    fn with_transport(self, transport: TransportKind) -> Self;
}

impl DescriptorTestExt for ModelDescriptor {
    fn with_transport(mut self, transport: TransportKind) -> Self {
        self.transport = transport;
        self
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

#[test]
fn list_models_data_serializes_oauth_anthropic_and_google_metadata() {
    let runtime: Arc<dyn LlmProvider> = Arc::new(ProviderRouter::with_model_descriptors(
        vec![provider("runtime")],
        vec![
            descriptor(
                "anthropic-api",
                "claude-oauth",
                AuthIdentity::OAuth {
                    provider: ProviderId::new("anthropic").unwrap(),
                },
            )
            .with_transport(TransportKind::AnthropicMessages),
            descriptor("google-api", "gemini", AuthIdentity::ApiKey)
                .with_transport(TransportKind::GoogleGenerativeAi),
        ],
    ));

    let data = list_models_data(&runtime);

    let models = data["models"].as_array().unwrap();
    let oauth = models
        .iter()
        .find(|model| model["model"] == "anthropic-api/claude-oauth")
        .unwrap();
    assert_eq!(oauth["api"], "anthropic-messages");
    assert_eq!(oauth["auth"], "oauth");
    assert_eq!(oauth["oauthProvider"], "anthropic");
    let google = models
        .iter()
        .find(|model| model["model"] == "google-api/gemini")
        .unwrap();
    assert_eq!(google["api"], "google-generative-ai");
}

#[test]
fn list_models_data_uses_direct_provider_descriptors_before_router_introspection() {
    let runtime = provider_with_descriptors(
        "direct",
        vec![descriptor("direct", "m", AuthIdentity::ApiKey)],
    );

    let data = list_models_data(&runtime);

    let models = data["models"].as_array().unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0]["provider"], "direct");
    assert_eq!(models[0]["model"], "direct/m");
}

#[test]
fn list_models_data_uses_retrying_provider_descriptor_surface_without_downcasts() {
    let child_descriptor = descriptor("child", "m", AuthIdentity::ApiKey);
    let child = provider_with_descriptors("child", vec![child_descriptor]);
    let router: Arc<dyn LlmProvider> = Arc::new(ProviderRouter::new(vec![child]));
    let runtime: Arc<dyn LlmProvider> =
        Arc::new(RetryingProvider::new(router, RetryConfig::no_delay(1)));

    let data = list_models_data(&runtime);

    let models = data["models"].as_array().unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0]["model"], "child/m");
}

#[test]
fn list_models_reports_a_failed_catalogue_reload_alongside_the_last_valid_list() {
    use super::list_catalogue_data;
    use crate::domain::catalogue::CatalogueSnapshot;

    let snapshot = CatalogueSnapshot::new(
        4,
        vec![descriptor("openai-api", "gpt-5", AuthIdentity::ApiKey)],
    );

    let healthy = list_catalogue_data(&snapshot, None);
    assert!(healthy.get("error").is_none());
    assert_eq!(healthy["models"].as_array().unwrap().len(), 1);

    let broken = list_catalogue_data(&snapshot, Some("failed to parse models.json"));
    assert_eq!(
        broken["error"].as_str(),
        Some("failed to parse models.json"),
        "a broken catalogue must not be projected as a healthy stale list"
    );
    assert_eq!(
        broken["models"].as_array().unwrap().len(),
        1,
        "the last valid generation stays listed"
    );
}
