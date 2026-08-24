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

struct Factory;

impl ProviderRuntimeFactory<Config> for Factory {
    fn compose_runtime(
        &self,
        config: &Config,
        base_dir: &std::path::Path,
        _http_client: &reqwest::Client,
    ) -> Result<Arc<dyn LlmProvider>, String> {
        assert_eq!(config.marker, "contract");
        assert!(base_dir.ends_with("workspace"));
        Ok(Arc::new(Provider))
    }
}

#[test]
fn provider_runtime_factory_composes_through_application_use_case() {
    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path().join("workspace");
    std::fs::create_dir(&workspace).unwrap();

    let provider = ComposeProviderRuntimeUseCase::new()
        .compose(
            &Factory,
            &Config { marker: "contract" },
            &workspace,
            &reqwest::Client::new(),
        )
        .unwrap();

    assert_eq!(provider.name(), "runtime-factory-provider");
}
