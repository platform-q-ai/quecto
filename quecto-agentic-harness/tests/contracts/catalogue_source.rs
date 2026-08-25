//! Contract coverage for catalogue source ports.

use quecto::catalogue_app::{CatalogueSource, ResolveCatalogueUseCase};
use quecto::domain::catalogue::{
    AuthIdentity, Availability, ModelCapabilities, ModelCost, ModelDescriptor, ModelRef,
    TransportKind,
};

fn descriptor(provider: &str, model: &str) -> ModelDescriptor {
    ModelDescriptor {
        reference: ModelRef::parse(provider, model).unwrap(),
        display_name: Some(model.to_string()),
        transport: TransportKind::OpenAiCompletions,
        auth: AuthIdentity::ApiKey,
        base_url: Some("https://example.test/v1".to_string()),
        auth_header: true,
        allow_remote_http: false,
        configured: true,
        capabilities: ModelCapabilities {
            input: vec!["text".to_string()],
            context_window: 1024,
            max_tokens: 256,
            context_window_explicit: true,
            max_tokens_explicit: true,
            reasoning: false,
            cost: ModelCost::default(),
        },
        availability: Availability::Runnable,
    }
}

struct StaticSource {
    name: &'static str,
    models: Vec<ModelDescriptor>,
}

impl CatalogueSource for StaticSource {
    fn name(&self) -> &str {
        self.name
    }

    fn load(&self) -> Result<Vec<ModelDescriptor>, String> {
        Ok(self.models.clone())
    }
}

struct FailingSource;

impl CatalogueSource for FailingSource {
    fn name(&self) -> &str {
        "broken"
    }

    fn load(&self) -> Result<Vec<ModelDescriptor>, String> {
        Err("boom".to_string())
    }
}

#[test]
fn catalogue_sources_are_resolved_in_declared_order_with_later_precedence() {
    let low = StaticSource {
        name: "low",
        models: vec![descriptor("provider", "model")],
    };
    let high = StaticSource {
        name: "high",
        models: vec![descriptor("provider", "model")],
    };

    let snapshot = ResolveCatalogueUseCase::new(vec![&low, &high])
        .resolve(9)
        .unwrap();

    assert_eq!(snapshot.generation, 9);
    assert_eq!(snapshot.models().len(), 1);
    assert_eq!(snapshot.models()[0].qualified_id(), "provider/model");
}

#[test]
fn catalogue_source_errors_are_attributed_to_the_source_name() {
    let err = ResolveCatalogueUseCase::new(vec![&FailingSource])
        .resolve(1)
        .unwrap_err();

    assert_eq!(err, "broken: boom");
}
