//! Contract coverage for the application-owned catalogue source port.

use quecto::catalogue_app::{CatalogueSource, ResolveCatalogueUseCase};
use quecto::domain::catalogue::{
    Availability, ModelCapabilities, ModelDescriptor, ModelId, ModelRef, ProviderId, TransportKind,
};

struct Layer {
    id: &'static str,
    models: Vec<ModelDescriptor>,
}

struct BrokenLayer;

impl CatalogueSource for Layer {
    fn id(&self) -> &str {
        self.id
    }

    fn load(&self) -> Result<Vec<ModelDescriptor>, String> {
        Ok(self.models.clone())
    }
}

impl CatalogueSource for BrokenLayer {
    fn id(&self) -> &str {
        "broken"
    }

    fn load(&self) -> Result<Vec<ModelDescriptor>, String> {
        Err("malformed source".to_string())
    }
}

fn descriptor(provider: &str, model: &str, display: &str) -> ModelDescriptor {
    ModelDescriptor {
        reference: ModelRef::new(
            ProviderId::new(provider).unwrap(),
            ModelId::new(model).unwrap(),
        ),
        display_name: Some(display.to_string()),
        transport: TransportKind::OpenAiCompletions,
        auth: quecto::domain::catalogue::AuthIdentity::ApiKey,
        base_url: None,
        auth_header: true,
        allow_remote_http: false,
        configured: true,
        capabilities: ModelCapabilities {
            input: Vec::new(),
            context_window: 0,
            max_tokens: 0,
            context_window_explicit: false,
            max_tokens_explicit: false,
            reasoning: false,
            cost: Default::default(),
        },
        availability: Availability::Runnable,
    }
}

#[test]
fn later_sources_override_earlier_ones_by_stable_identity_and_keep_position() {
    let builtin = Layer {
        id: "builtin",
        models: vec![
            descriptor("p", "a", "builtin a"),
            descriptor("p", "b", "builtin b"),
        ],
    };
    let user = Layer {
        id: "user",
        models: vec![
            descriptor("p", "b", "user b"),
            descriptor("p", "c", "user c"),
        ],
    };

    let resolved = ResolveCatalogueUseCase.resolve_sources(7, &[&builtin, &user]);

    assert!(resolved.skipped.is_empty());
    assert_eq!(resolved.snapshot.generation, 7);
    let listed: Vec<_> = resolved
        .snapshot
        .models()
        .iter()
        .map(|model| (model.qualified_id(), model.display_name.clone().unwrap()))
        .collect();
    assert_eq!(
        listed,
        vec![
            ("p/a".to_string(), "builtin a".to_string()),
            ("p/b".to_string(), "user b".to_string()),
            ("p/c".to_string(), "user c".to_string()),
        ]
    );
}

#[test]
fn a_failing_source_is_reported_and_skipped_without_discarding_the_other_layers() {
    let builtin = Layer {
        id: "builtin",
        models: vec![descriptor("p", "a", "builtin a")],
    };

    let resolved = ResolveCatalogueUseCase.resolve_sources(1, &[&builtin, &BrokenLayer]);

    assert_eq!(resolved.snapshot.models().len(), 1);
    assert_eq!(resolved.skipped.len(), 1);
    assert_eq!(resolved.skipped[0].source, "broken");
    assert!(resolved.skipped[0].error.contains("malformed source"));
}
