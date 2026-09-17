use super::*;
use crate::application::configuration::dto::ConfigSelection;
use crate::application::configuration::dto::{ConfigLayers, EffectiveConfigError};
use crate::application::configuration::use_cases::fakes::{FakeTrust, FakeValidator, MemoryStore};
use serde_json::json;
use std::path::PathBuf;

const GLOBAL: &str = "/home/u/.quecto/config.json";
const OVERLAY: &str = "/work/.quecto/config.json";

fn layered() -> ConfigSelection {
    ConfigSelection::Layered(ConfigLayers {
        global: PathBuf::from(GLOBAL),
        overlay: Some(PathBuf::from(OVERLAY)),
        legacy_local: None,
    })
}

fn request(
    selection: ConfigSelection,
    scope: ConfigReadScope,
    key_path: Option<&str>,
) -> ConfigReadRequest {
    ConfigReadRequest {
        selection,
        scope,
        key_path: key_path.map(str::to_string),
        reveal_secrets: true,
    }
}

fn use_case(store: Arc<MemoryStore>, trust: Arc<FakeTrust>) -> ReadConfiguration {
    let resolve = Arc::new(ResolveEffectiveConfig::new(
        store.clone(),
        Arc::new(FakeValidator::default()),
        trust,
    ));
    ReadConfiguration::new(store, resolve)
}

#[test]
fn each_scope_reads_its_own_document() {
    let overlay = r#"{"agents":{"defaults":{"model":"local"}}}"#;
    let store = MemoryStore::with(&[
        (
            GLOBAL,
            r#"{"agents":{"defaults":{"model":"global","effort":"high"}}}"#,
        ),
        (OVERLAY, overlay),
    ]);
    let read = use_case(store, FakeTrust::trusting(OVERLAY, overlay));
    let effective = read
        .execute(request(
            layered(),
            ConfigReadScope::Effective,
            Some("agents.defaults"),
        ))
        .unwrap();
    assert_eq!(effective.value, json!({"model":"local","effort":"high"}));
    assert!(effective.sources.unwrap().applied_overlay().is_some());

    let global = read
        .execute(request(
            layered(),
            ConfigReadScope::Global,
            Some("agents.defaults.model"),
        ))
        .unwrap();
    assert_eq!(global.value, json!("global"));
    assert_eq!(global.sources, None);

    let local = read
        .execute(request(layered(), ConfigReadScope::Overlay, None))
        .unwrap();
    assert_eq!(
        local.value,
        json!({"agents":{"defaults":{"model":"local"}}})
    );
}

#[test]
fn the_overlay_scope_reads_the_file_as_written_even_when_untrusted() {
    let store = MemoryStore::with(&[(OVERLAY, r#"{"agents":{"defaults":{"model":"local"}}}"#)]);
    let read = use_case(store, Arc::new(FakeTrust::default()));
    let local = read
        .execute(request(
            layered(),
            ConfigReadScope::Overlay,
            Some("agents.defaults.model"),
        ))
        .unwrap();
    assert_eq!(local.value, json!("local"));
    let effective = read
        .execute(request(
            layered(),
            ConfigReadScope::Effective,
            Some("agents.defaults.model"),
        ))
        .unwrap_err();
    assert_eq!(
        effective,
        ConfigReadError::NotSet("agents.defaults.model".into())
    );
}

#[test]
fn absent_files_read_as_empty_objects_and_missing_keys_are_not_set() {
    let read = use_case(MemoryStore::with(&[]), Arc::new(FakeTrust::default()));
    assert_eq!(
        read.execute(request(layered(), ConfigReadScope::Global, None))
            .unwrap()
            .value,
        json!({})
    );
    assert_eq!(
        read.execute(request(layered(), ConfigReadScope::Overlay, None))
            .unwrap()
            .value,
        json!({})
    );
    let error = read
        .execute(request(
            layered(),
            ConfigReadScope::Global,
            Some("agents.defaults.model"),
        ))
        .unwrap_err();
    assert_eq!(error.to_string(), "`agents.defaults.model` is not set");
}

#[test]
fn an_explicit_selection_has_no_overlay_to_read() {
    let read = use_case(
        MemoryStore::with(&[("/x/c.json", "{}")]),
        Arc::new(FakeTrust::default()),
    );
    let selection = ConfigSelection::Explicit(PathBuf::from("/x/c.json"));
    assert_eq!(
        read.execute(request(selection.clone(), ConfigReadScope::Overlay, None))
            .unwrap_err(),
        ConfigReadError::NoOverlayLocation
    );
    assert_eq!(
        read.execute(request(selection, ConfigReadScope::Global, None))
            .unwrap()
            .value,
        json!({})
    );
}

#[test]
fn failures_are_reported_per_scope() {
    let store = MemoryStore::with(&[(GLOBAL, "{ nope")]);
    let read = use_case(store, Arc::new(FakeTrust::default()));
    assert!(matches!(
        read.execute(request(layered(), ConfigReadScope::Global, None))
            .unwrap_err(),
        ConfigReadError::Parse { .. }
    ));
    let error = read
        .execute(request(layered(), ConfigReadScope::Effective, None))
        .unwrap_err();
    assert!(matches!(
        error,
        ConfigReadError::Effective(EffectiveConfigError::Parse { .. })
    ));
    assert!(error.to_string().starts_with("failed to load config"));

    let mut store = MemoryStore::default();
    store.broken.insert(PathBuf::from(GLOBAL));
    let read = use_case(Arc::new(store), Arc::new(FakeTrust::default()));
    let error = read
        .execute(request(layered(), ConfigReadScope::Global, None))
        .unwrap_err();
    assert!(matches!(error, ConfigReadError::Read { .. }));
    assert!(error.to_string().contains("permission denied"));
    assert!(
        ConfigReadError::NoOverlayLocation
            .to_string()
            .contains("--config")
    );
    assert!(format!("{read:?}").contains("ReadConfiguration"));
}

#[test]
fn secrets_are_redacted_unless_revealed_in_every_scope_and_for_a_bare_leaf() {
    let global = r#"{"providers":{"openai":{"api_key":"sk-global","api_base":"https://x"}}}"#;
    let overlay = r#"{"agents":{"defaults":{"model":"local","refresh_token":"t"}}}"#;
    let store = MemoryStore::with(&[(GLOBAL, global), (OVERLAY, overlay)]);
    let read = use_case(store, FakeTrust::trusting(OVERLAY, overlay));
    let hidden = |scope, key_path: Option<&str>| ConfigReadRequest {
        reveal_secrets: false,
        ..request(layered(), scope, key_path)
    };

    let effective = read
        .execute(hidden(ConfigReadScope::Effective, None))
        .unwrap();
    assert_eq!(effective.redacted, 2);
    assert_eq!(
        effective.value["providers"]["openai"]["api_key"],
        "<redacted>"
    );
    assert_eq!(
        effective.value["providers"]["openai"]["api_base"],
        "https://x"
    );
    assert_eq!(
        effective.value["agents"]["defaults"]["refresh_token"],
        "<redacted>"
    );

    let leaf = read
        .execute(hidden(
            ConfigReadScope::Global,
            Some("providers.openai.api_key"),
        ))
        .unwrap();
    assert_eq!(leaf.value, json!("<redacted>"));
    assert_eq!(leaf.redacted, 1);

    let model = read
        .execute(hidden(
            ConfigReadScope::Overlay,
            Some("agents.defaults.model"),
        ))
        .unwrap();
    assert_eq!(model.value, json!("local"));
    assert_eq!(model.redacted, 0);

    let revealed = read
        .execute(request(
            layered(),
            ConfigReadScope::Global,
            Some("providers.openai.api_key"),
        ))
        .unwrap();
    assert_eq!(revealed.value, json!("sk-global"));
    assert_eq!(revealed.redacted, 0);
}
