use std::sync::Arc;

use super::*;
use crate::domain::catalogue::CatalogueSnapshot;
use crate::domain::message::LlmResponse;
use crate::domain::provider::{ChatRequest, StreamEvent};

#[derive(Debug)]
struct NamedProvider;

impl LlmProvider for NamedProvider {
    fn name(&self) -> &str {
        "composed"
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

struct CapturingComposer;

impl CatalogueRuntimeComposer for CapturingComposer {
    fn compose(&self, snapshot: &CatalogueSnapshot) -> Result<Arc<dyn LlmProvider>, String> {
        assert_eq!(snapshot.generation, 42);
        assert!(snapshot.models().is_empty());
        Ok(Arc::new(NamedProvider))
    }
}

#[test]
fn compose_catalogue_runtime_preserves_catalogue_generation_with_provider_runtime() {
    let snapshot = CatalogueSnapshot::empty(42);
    let runtime = ComposeCatalogueRuntimeUseCase::new(&CapturingComposer)
        .compose(snapshot)
        .unwrap();

    assert_eq!(runtime.generation(), 42);
    assert_eq!(runtime.provider.name(), "composed");
}
