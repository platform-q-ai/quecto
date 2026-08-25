//! Contract coverage for provider runtime factory ports.

use std::sync::Arc;

use quecto::domain::message::LlmResponse;
use quecto::domain::provider::{ChatRequest, LlmProvider, StreamEvent};
use quecto::provider_runtime_app::{ComposeProviderRuntimeUseCase, ProviderRuntimeFactory};

#[derive(Debug)]
struct Provider;

impl LlmProvider for Provider {
    fn name(&self) -> &str {
        "runtime-factory-provider"
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
}

struct Config {
    marker: &'static str,
}

struct RuntimeInputs {
    workspace_name: &'static str,
    client_marker: &'static str,
}

struct Factory;

impl ProviderRuntimeFactory<Config, RuntimeInputs> for Factory {
    fn compose_runtime(
        &self,
        config: &Config,
        runtime_inputs: &RuntimeInputs,
    ) -> Result<Arc<dyn LlmProvider>, String> {
        assert_eq!(config.marker, "contract");
        assert_eq!(runtime_inputs.workspace_name, "workspace");
        assert_eq!(runtime_inputs.client_marker, "test-client");
        Ok(Arc::new(Provider))
    }
}

#[test]
fn provider_runtime_factory_composes_through_application_use_case() {
    let runtime_inputs = RuntimeInputs {
        workspace_name: "workspace",
        client_marker: "test-client",
    };

    let provider = ComposeProviderRuntimeUseCase::new()
        .compose(&Factory, &Config { marker: "contract" }, &runtime_inputs)
        .unwrap();

    assert_eq!(provider.name(), "runtime-factory-provider");
}
