use super::*;
use crate::application::catalogue::CatalogueSourceError;
use crate::application::catalogue::dto::{ListedModel, ListingDiagnostic, ModelCatalogueListing};
use crate::domain::catalogue::{
    Availability, CatalogueEntry, ModelCapabilities, ModelCost, ModelDescriptor, ModelId, ModelRef,
    ProviderDescriptor, ProviderId, TransportKind,
};

fn entry(oauth: bool) -> CatalogueEntry {
    CatalogueEntry {
        provider: ProviderDescriptor {
            id: ProviderId::new("acme").unwrap(),
            display_name: None,
            transport: TransportKind::OpenAiCompletions,
            auth: if oauth {
                AuthIdentity::OAuth {
                    provider: Some(ProviderId::new("openai").unwrap()),
                }
            } else {
                AuthIdentity::ApiKey
            },
        },
        model: ModelDescriptor {
            reference: ModelRef::new(
                ProviderId::new("acme").unwrap(),
                ModelId::new("m1").unwrap(),
            ),
            display_name: Some("Model One".into()),
            capabilities: ModelCapabilities {
                effort_levels: vec!["low".into(), "high".into()],
                input_modalities: vec!["text".into(), "image".into()],
                context_window: 200_000,
                max_output_tokens: 8192,
                context_window_explicit: true,
                max_output_tokens_explicit: true,
                reasoning: true,
                cost: ModelCost {
                    input: 1.0,
                    output: 2.0,
                    cache_read: 0.25,
                    cache_write: 0.5,
                },
            },
            availability: Availability::runnable(),
        },
    }
}

#[test]
fn renders_every_legacy_field_from_the_listed_entry() {
    let listing = ModelCatalogueListing {
        generation: 7,
        models: vec![ListedModel {
            entry: entry(true),
            runnable: false,
        }],
        rejected: vec![ListingDiagnostic {
            model: "acme/bad".into(),
            reason: "models.json: invalid".into(),
        }],
    };
    let json = render(&ModelListingOutcome::Listed(listing));
    assert_eq!(json["generation"], 7);
    assert_eq!(json["rejected"][0]["model"], "acme/bad");
    assert_eq!(json["rejected"][0]["reason"], "models.json: invalid");
    let m = &json["models"][0];
    assert_eq!(m["provider"], "acme");
    assert_eq!(m["id"], "m1");
    assert_eq!(m["model"], "acme/m1");
    assert_eq!(m["name"], "Model One");
    assert_eq!(m["api"], "openai-completions");
    assert_eq!(m["auth"], "oauth");
    assert_eq!(m["oauthProvider"], "openai");
    assert_eq!(m["contextWindow"], 200_000);
    assert_eq!(m["maxTokens"], 8192);
    assert_eq!(m["input"], serde_json::json!(["text", "image"]));
    assert_eq!(m["cost"]["cacheRead"], 0.25);
    assert_eq!(m["reasoning"], true);
    assert_eq!(m["effortLevels"], serde_json::json!(["low", "high"]));
    assert_eq!(
        m["configured"], false,
        "configured is the use case's runnable verdict"
    );
}

#[test]
fn api_key_auth_has_no_oauth_provider() {
    let listing = ModelCatalogueListing {
        generation: 1,
        models: vec![ListedModel {
            entry: entry(false),
            runnable: true,
        }],
        rejected: vec![],
    };
    let json = render(&ModelListingOutcome::Listed(listing));
    assert_eq!(json["models"][0]["auth"], "apiKey");
    assert!(json["models"][0]["oauthProvider"].is_null());
    assert_eq!(json["models"][0]["configured"], true);
}

#[test]
fn a_failed_source_renders_no_models_and_the_error() {
    let json = render(&ModelListingOutcome::SourceUnavailable(
        CatalogueSourceError {
            source: "models.json".into(),
            error: "expected value at line 1".into(),
        },
    ));
    assert_eq!(
        json,
        serde_json::json!({ "models": [], "error": "expected value at line 1" })
    );
}
