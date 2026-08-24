//! Contract coverage for catalogue runtime composition ports.

use std::sync::Arc;

use quecto::catalogue_runtime_app::{CatalogueRuntimeComposer, ComposeCatalogueRuntimeUseCase};
use quecto::domain::catalogue::CatalogueSnapshot;
use quecto::domain::message::LlmResponse;
use quecto::domain::provider::{ChatRequest, LlmProvider, StreamEvent};

#[derive(Debug)]
struct Provider;

impl LlmProvider for Provider {
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
}

struct Composer;

impl CatalogueRuntimeComposer for Composer {
    fn compose(&self, snapshot: &CatalogueSnapshot) -> Result<Arc<dyn LlmProvider>, String> {
        assert_eq!(snapshot.generation, 77);
        Ok(Arc::new(Provider))
    }
}

#[test]
fn catalogue_runtime_composer_preserves_generation_with_runtime() {
    let runtime = ComposeCatalogueRuntimeUseCase::new(&Composer)
        .compose(CatalogueSnapshot::empty(77))
        .unwrap();

    assert_eq!(runtime.generation(), 77);
    assert_eq!(runtime.provider.name(), "contract-provider");
}
