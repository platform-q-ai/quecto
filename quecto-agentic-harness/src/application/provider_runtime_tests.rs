use std::sync::Arc;

use super::*;
use crate::domain::message::LlmResponse;
use crate::domain::provider::{ChatRequest, StreamEvent};
use crate::infrastructure::config::Config;

#[derive(Debug)]
struct StubProvider;

impl LlmProvider for StubProvider {
    fn name(&self) -> &str {
        "stub-runtime"
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
            dyn std::future::Future<Output = Result<LlmResponse, crate::domain::error::DomainError>>
                + Send
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

struct RuntimeInputs {
    root_name: &'static str,
    client_marker: &'static str,
}

struct CapturingFactory;

impl ProviderRuntimeFactory<Config, RuntimeInputs> for CapturingFactory {
    fn compose_runtime(
        &self,
        config: &Config,
        runtime_inputs: &RuntimeInputs,
    ) -> Result<Arc<dyn LlmProvider>, String> {
        assert_eq!(config.providers.openai.api_key, "sk-test");
        assert_eq!(runtime_inputs.root_name, "runtime-root");
        assert_eq!(runtime_inputs.client_marker, "test-client");
        Ok(Arc::new(StubProvider))
    }
}

#[test]
fn compose_provider_runtime_use_case_delegates_through_application_owned_port() {
    let mut config = Config::default();
    config.providers.openai.api_key = "sk-test".to_string();
    let runtime_inputs = RuntimeInputs {
        root_name: "runtime-root",
        client_marker: "test-client",
    };

    let provider = ComposeProviderRuntimeUseCase::new()
        .compose(&CapturingFactory, &config, &runtime_inputs)
        .unwrap();

    assert_eq!(provider.name(), "stub-runtime");
}
