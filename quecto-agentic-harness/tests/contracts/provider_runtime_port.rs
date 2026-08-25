//! Contract coverage for the application-owned provider runtime port.

use std::sync::Arc;

use quecto::catalogue_runtime_app::CatalogueRuntimeSnapshot;
use quecto::domain::catalogue::{
    AuthIdentity, Availability, ModelCapabilities, ModelCost, ModelDescriptor, ModelRef,
    TransportKind,
};
use quecto::domain::message::LlmResponse;
use quecto::domain::provider::{ChatRequest, LlmProvider, StreamEvent};
use quecto::provider_runtime_app::{ProviderRuntimeApplication, ProviderRuntimePort};

#[derive(Debug)]
struct NamedProvider {
    descriptors: Vec<ModelDescriptor>,
}

impl LlmProvider for NamedProvider {
    fn name(&self) -> &str {
        "contract-provider"
    }

    fn chat<'a>(
        &'a self,
        _request: ChatRequest<'a>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<LlmResponse, quecto::domain::error::DomainError>,
                > + Send
                + 'a,
        >,
    > {
        Box::pin(async {
            Ok(LlmResponse {
                content: Some("ok".to_string()),
                tool_calls: vec![],
                usage: None,
                stop_reason: None,
                thinking_blocks: vec![],
            })
        })
    }

    fn chat_stream<'a>(
        &'a self,
        request: ChatRequest<'a>,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<LlmResponse, quecto::domain::error::DomainError>,
                > + Send
                + 'a,
        >,
    > {
        self.chat(request)
    }

    fn chat_stream_incremental<'a>(
        &'a self,
        _request: ChatRequest<'a>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = tokio::sync::mpsc::Receiver<StreamEvent>> + Send + 'a>,
    > {
        Box::pin(async {
            let (_tx, rx) = tokio::sync::mpsc::channel(1);
            rx
        })
    }

    fn model_descriptors(&self) -> Option<&[ModelDescriptor]> {
        Some(&self.descriptors)
    }
}

fn sample_descriptor() -> ModelDescriptor {
    ModelDescriptor {
        reference: ModelRef::parse("open", "alpha").unwrap(),
        display_name: None,
        transport: TransportKind::OpenAiCompletions,
        auth: AuthIdentity::ApiKey,
        base_url: None,
        auth_header: false,
        allow_remote_http: false,
        configured: true,
        capabilities: ModelCapabilities {
            input: Vec::new(),
            context_window: 0,
            max_tokens: 0,
            context_window_explicit: false,
            max_tokens_explicit: false,
            reasoning: false,
            cost: ModelCost {
                input: 0.0,
                output: 0.0,
                cache_read: 0.0,
                cache_write: 0.0,
            },
        },
        availability: Availability::Runnable,
    }
}

struct FakePort;

impl ProviderRuntimePort<(), ()> for FakePort {
    fn compose(&self, _config: &(), _inputs: &()) -> Result<Arc<dyn LlmProvider>, String> {
        Ok(Arc::new(NamedProvider {
            descriptors: vec![sample_descriptor()],
        }))
    }
}

#[test]
fn provider_runtime_port_composes_through_the_application() {
    let provider = ProviderRuntimeApplication::new(FakePort)
        .compose(&(), &())
        .unwrap();

    assert_eq!(provider.name(), "contract-provider");
}

#[test]
fn provider_runtime_port_reload_publishes_catalogue_runtime_snapshot() {
    let runtime: CatalogueRuntimeSnapshot = ProviderRuntimeApplication::new(FakePort)
        .compose_reload(&(), &())
        .unwrap();

    assert_eq!(runtime.generation(), 0);
    assert_eq!(runtime.provider.name(), "contract-provider");
    assert_eq!(runtime.catalogue.models()[0].qualified_id(), "open/alpha");
}
