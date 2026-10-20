use std::path::PathBuf;
use std::sync::Mutex;

use serde_json::Value;

use super::*;
use crate::application::configuration::dto::{ConfigLayers, EffectiveConfigError, OverlayState};
use crate::application::configuration::ports::{
    RunSettings, StoredCredential, StoredCredentialsUnavailable,
};
use crate::application::configuration::use_cases::fakes::{FakeTrust, FakeValidator, MemoryStore};

const GLOBAL: &str = "/home/u/.quecto/config.json";
const OVERLAY: &str = "/work/.quecto/config.json";

fn layered() -> ConfigSelection {
    ConfigSelection::Layered(ConfigLayers {
        global: PathBuf::from(GLOBAL),
        overlay: Some(PathBuf::from(OVERLAY)),
        legacy_local: None,
        overlay_origin: Default::default(),
    })
}

/// Realizes the model, effort and keys straight from the document,
/// recording what it was handed.
#[derive(Default)]
struct FakeSettings {
    seen: Mutex<Vec<Value>>,
    refusal: Option<String>,
}

impl RunSettingsSource for FakeSettings {
    fn settings(&self, document: Value) -> Result<RunSettings, String> {
        self.seen.lock().unwrap().push(document.clone());
        if let Some(reason) = &self.refusal {
            return Err(reason.clone());
        }
        let text = |pointer: &str| {
            document
                .pointer(pointer)
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        Ok(RunSettings {
            workspace: "/work".to_string(),
            model: text("/agents/defaults/model").unwrap_or_else(|| "default-model".to_string()),
            effort: text("/agents/defaults/effort"),
            openai_key: text("/providers/openai/api_key").is_some(),
            anthropic_key: text("/providers/anthropic/api_key").is_some(),
            endpoint_keys: Vec::new(),
        })
    }
}

struct FakeCredentials(Result<Vec<StoredCredential>, StoredCredentialsUnavailable>);

impl StoredCredentialPresence for FakeCredentials {
    fn stored(&self) -> Result<Vec<StoredCredential>, StoredCredentialsUnavailable> {
        self.0.clone()
    }
}

fn stored(provider: &str) -> StoredCredential {
    StoredCredential {
        provider: provider.to_string(),
        method: "oauth".to_string(),
        state: "active".to_string(),
    }
}

fn inspect(
    store: Arc<MemoryStore>,
    trust: Arc<FakeTrust>,
    settings: Arc<FakeSettings>,
    credentials: Result<Vec<StoredCredential>, StoredCredentialsUnavailable>,
) -> InspectEffectiveConfiguration {
    let resolve = Arc::new(ResolveEffectiveConfig::new(
        store,
        Arc::new(FakeValidator::default()),
        trust,
    ));
    InspectEffectiveConfiguration::new(resolve, settings, Arc::new(FakeCredentials(credentials)))
}

#[test]
fn the_effective_document_is_inspected_with_its_sources_and_stored_credentials() {
    let overlay = r#"{"agents":{"defaults":{"model":"local"}}}"#;
    let store = MemoryStore::with(&[
        (
            GLOBAL,
            r#"{"agents":{"defaults":{"model":"global","effort":"high"}},"providers":{"openai":{"api_key":"fake-key"}}}"#,
        ),
        (OVERLAY, overlay),
    ]);
    let settings = Arc::new(FakeSettings::default());
    let inspection = inspect(
        store,
        FakeTrust::trusting(OVERLAY, overlay),
        Arc::clone(&settings),
        Ok(vec![stored("xai")]),
    )
    .execute(&layered())
    .unwrap();
    assert_eq!(
        inspection.settings.model, "local",
        "the trusted overlay wins"
    );
    assert_eq!(inspection.settings.effort.as_deref(), Some("high"));
    assert!(inspection.settings.openai_key);
    assert!(!inspection.settings.anthropic_key);
    assert_eq!(inspection.stored_credentials, Ok(vec![stored("xai")]));
    assert_eq!(inspection.sources.base, PathBuf::from(GLOBAL));
    assert_eq!(
        inspection.sources.overlay.unwrap().state,
        OverlayState::Applied
    );
    assert_eq!(settings.seen.lock().unwrap().len(), 1);
}

#[test]
fn an_untrusted_overlay_is_reported_and_not_applied() {
    let store = MemoryStore::with(&[
        (GLOBAL, r#"{"agents":{"defaults":{"model":"global"}}}"#),
        (OVERLAY, r#"{"agents":{"defaults":{"model":"local"}}}"#),
    ]);
    let inspection = inspect(
        store,
        Arc::new(FakeTrust::default()),
        Arc::new(FakeSettings::default()),
        Ok(Vec::new()),
    )
    .execute(&layered())
    .unwrap();
    assert_eq!(inspection.settings.model, "global");
    assert!(matches!(
        inspection.sources.overlay.unwrap().state,
        OverlayState::Untrusted { .. }
    ));
}

#[test]
fn an_unreadable_credential_store_does_not_hide_the_configuration() {
    let store = MemoryStore::with(&[(GLOBAL, "{}")]);
    let inspection = inspect(
        store,
        Arc::new(FakeTrust::default()),
        Arc::new(FakeSettings::default()),
        Err(StoredCredentialsUnavailable::Malformed),
    )
    .execute(&layered())
    .unwrap();
    assert_eq!(inspection.settings.model, "default-model");
    assert_eq!(
        inspection.stored_credentials,
        Err(StoredCredentialsUnavailable::Malformed)
    );
}

#[test]
fn a_layer_that_cannot_be_resolved_refuses_before_realizing() {
    let store = Arc::new(MemoryStore {
        broken: [PathBuf::from(GLOBAL)].into_iter().collect(),
        ..MemoryStore::default()
    });
    store
        .files
        .lock()
        .unwrap()
        .insert(PathBuf::from(GLOBAL), b"{}".to_vec());
    let settings = Arc::new(FakeSettings::default());
    let refused = inspect(
        store,
        Arc::new(FakeTrust::default()),
        Arc::clone(&settings),
        Ok(Vec::new()),
    )
    .execute(&layered())
    .unwrap_err();
    assert!(
        matches!(
            refused,
            InspectionError::Effective(EffectiveConfigError::Read { .. })
        ),
        "{refused:?}"
    );
    assert!(settings.seen.lock().unwrap().is_empty());
}

#[test]
fn a_document_that_does_not_realize_refuses_in_the_schemas_words() {
    let store = MemoryStore::with(&[(GLOBAL, r#"{"agents":{"defaults":{"model":42}}}"#)]);
    let settings = Arc::new(FakeSettings {
        refusal: Some("invalid type: integer `42`, expected a string".to_string()),
        ..FakeSettings::default()
    });
    let refused = inspect(
        store,
        Arc::new(FakeTrust::default()),
        settings,
        Ok(Vec::new()),
    )
    .execute(&layered())
    .unwrap_err();
    assert_eq!(
        refused,
        InspectionError::Settings("invalid type: integer `42`, expected a string".to_string())
    );
    assert_eq!(
        refused.to_string(),
        "invalid type: integer `42`, expected a string"
    );
}

#[test]
fn inspecting_never_writes() {
    let store = MemoryStore::with(&[(GLOBAL, "{}")]);
    inspect(
        Arc::clone(&store),
        Arc::new(FakeTrust::default()),
        Arc::new(FakeSettings::default()),
        Ok(Vec::new()),
    )
    .execute(&layered())
    .unwrap();
    assert!(store.returned.lock().unwrap().is_empty());
    assert_eq!(store.content(GLOBAL).as_deref(), Some("{}"));
}
