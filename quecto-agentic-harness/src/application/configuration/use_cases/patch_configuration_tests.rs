use super::*;
use crate::application::configuration::dto::ConfigTarget;
use crate::application::configuration::use_cases::fakes::{FakeTrust, FakeValidator, MemoryStore};
use serde_json::json;
use std::path::PathBuf;

const GLOBAL: &str = "/home/u/.quecto/config.json";
const OVERLAY: &str = "/work/.quecto/config.json";

fn patch(layer: ConfigLayer, path: &str, key_path: &str, value: Value) -> ConfigPatch {
    ConfigPatch {
        target: ConfigTarget {
            layer,
            path: PathBuf::from(path),
        },
        key_path: key_path.into(),
        value,
    }
}

fn use_case(store: Arc<MemoryStore>, trust: Arc<FakeTrust>) -> PatchConfiguration {
    PatchConfiguration::new(
        store.clone(),
        store,
        Arc::new(FakeValidator::default()),
        trust,
    )
}

fn document(store: &MemoryStore, path: &str) -> Value {
    serde_json::from_str(&store.content(path).unwrap()).unwrap()
}

#[test]
fn only_the_addressed_path_changes_and_unknown_keys_and_order_survive() {
    let store = MemoryStore::with(&[(
        GLOBAL,
        r#"{"zeta":"kept","agents":{"defaults":{"model":"old","effort":"high"}},"alpha":1}"#,
    )]);
    let receipt = use_case(store.clone(), Arc::new(FakeTrust::default()))
        .execute(patch(
            ConfigLayer::Global,
            GLOBAL,
            "agents.defaults.model",
            json!("new"),
        ))
        .unwrap();
    assert_eq!(
        receipt,
        ConfigPatchReceipt {
            path: PathBuf::from(GLOBAL),
            created: false
        }
    );
    assert_eq!(
        store.content(GLOBAL).unwrap(),
        "{\"zeta\":\"kept\",\"agents\":{\"defaults\":{\"model\":\"new\",\"effort\":\"high\"}},\"alpha\":1}\n"
    );
}

#[test]
fn a_missing_file_is_created_with_only_the_patched_path() {
    let store = MemoryStore::with(&[]);
    let receipt = use_case(store.clone(), Arc::new(FakeTrust::default()))
        .execute(patch(
            ConfigLayer::Global,
            GLOBAL,
            "agents.defaults.model",
            json!("m"),
        ))
        .unwrap();
    assert!(receipt.created);
    assert_eq!(
        document(&store, GLOBAL),
        json!({"agents":{"defaults":{"model":"m"}}})
    );
}

#[test]
fn an_invalid_result_is_refused_before_anything_is_written() {
    let original = r#"{"agents":{"defaults":{"effort":"high"}}}"#;
    let store = MemoryStore::with(&[(GLOBAL, original)]);
    let error = use_case(store.clone(), Arc::new(FakeTrust::default()))
        .execute(patch(
            ConfigLayer::Global,
            GLOBAL,
            "agents.defaults.effort",
            json!("bogus"),
        ))
        .unwrap_err();
    assert_eq!(
        error,
        ConfigPatchError::Invalid {
            path: PathBuf::from(GLOBAL),
            reason: "invalid effort level 'bogus'".into()
        }
    );
    assert!(error.to_string().contains("invalid effort level"));
    assert_eq!(store.content(GLOBAL).unwrap(), original, "untouched");

    let error = use_case(store.clone(), Arc::new(FakeTrust::default()))
        .execute(patch(
            ConfigLayer::Global,
            GLOBAL,
            "container_scripts",
            json!({}),
        ))
        .unwrap_err();
    assert!(
        matches!(error, ConfigPatchError::Invalid { .. }),
        "resolve failures count: {error}"
    );
    assert_eq!(store.content(GLOBAL).unwrap(), original, "untouched");
}

#[test]
fn key_paths_must_be_dotted_object_keys() {
    let store = MemoryStore::with(&[(
        GLOBAL,
        r#"{"agents":{"defaults":{"model":"m"}},"list":[1]}"#,
    )]);
    let use_case = use_case(store.clone(), Arc::new(FakeTrust::default()));
    for bad in ["", "a..b", ".a", "a."] {
        assert_eq!(
            use_case
                .execute(patch(ConfigLayer::Global, GLOBAL, bad, json!(1)))
                .unwrap_err(),
            ConfigPatchError::InvalidKeyPath(bad.into())
        );
    }
    let error = use_case
        .execute(patch(
            ConfigLayer::Global,
            GLOBAL,
            "agents.defaults.model.x",
            json!(1),
        ))
        .unwrap_err();
    assert_eq!(
        error,
        ConfigPatchError::NotAnObject {
            path: PathBuf::from(GLOBAL),
            at: "agents.defaults.model".into()
        }
    );
    assert!(error.to_string().contains("`agents.defaults.model`"));
    assert_eq!(
        use_case
            .execute(patch(ConfigLayer::Global, GLOBAL, "list.0", json!(1)))
            .unwrap_err(),
        ConfigPatchError::NotAnObject {
            path: PathBuf::from(GLOBAL),
            at: "list".into()
        }
    );
    let store = MemoryStore::with(&[(GLOBAL, "[1]")]);
    assert_eq!(
        super::PatchConfiguration::new(
            store.clone(),
            store,
            Arc::new(FakeValidator::default()),
            Arc::new(FakeTrust::default())
        )
        .execute(patch(ConfigLayer::Global, GLOBAL, "a", json!(1)))
        .unwrap_err(),
        ConfigPatchError::NotAnObject {
            path: PathBuf::from(GLOBAL),
            at: String::new()
        }
    );
}

#[test]
fn read_parse_and_write_failures_are_reported() {
    let mut store = MemoryStore::default();
    store.broken.insert(PathBuf::from(GLOBAL));
    assert!(matches!(
        use_case(Arc::new(store), Arc::new(FakeTrust::default()))
            .execute(patch(ConfigLayer::Global, GLOBAL, "a", json!(1)))
            .unwrap_err(),
        ConfigPatchError::Read { .. }
    ));
    let store = MemoryStore::with(&[(GLOBAL, "{ nope")]);
    assert!(matches!(
        use_case(store, Arc::new(FakeTrust::default()))
            .execute(patch(ConfigLayer::Global, GLOBAL, "a", json!(1)))
            .unwrap_err(),
        ConfigPatchError::Parse { .. }
    ));
    let store = MemoryStore {
        fail_writes: true,
        ..Default::default()
    };
    let error = use_case(Arc::new(store), Arc::new(FakeTrust::default()))
        .execute(patch(ConfigLayer::Global, GLOBAL, "a", json!(1)))
        .unwrap_err();
    assert_eq!(
        error,
        ConfigPatchError::Write {
            path: PathBuf::from(GLOBAL),
            reason: "disk full".into()
        }
    );
}

#[test]
fn an_overlay_patch_refuses_global_only_keys_before_reading() {
    let store = MemoryStore::with(&[]);
    let error = use_case(store.clone(), Arc::new(FakeTrust::default()))
        .execute(patch(
            ConfigLayer::Overlay,
            OVERLAY,
            "providers.openai.api_key",
            json!("k"),
        ))
        .unwrap_err();
    assert_eq!(
        error,
        ConfigPatchError::GlobalOnlyKey {
            path: PathBuf::from(OVERLAY),
            key: "providers".into()
        }
    );
    assert!(error.to_string().contains("`providers` is global-only"));
    assert_eq!(store.content(OVERLAY), None);
    assert!(
        use_case(store.clone(), Arc::new(FakeTrust::default()))
            .execute(patch(
                ConfigLayer::Global,
                GLOBAL,
                "providers.openai.api_key",
                json!("k")
            ))
            .is_ok(),
        "the global file may carry it"
    );
}

#[test]
fn an_overlay_patch_creates_the_file_and_records_trust_for_the_bytes_written() {
    let store = MemoryStore::with(&[]);
    let trust = Arc::new(FakeTrust::default());
    let receipt = use_case(store.clone(), trust.clone())
        .execute(patch(
            ConfigLayer::Overlay,
            OVERLAY,
            "agents.defaults.model",
            json!("m"),
        ))
        .unwrap();
    assert!(receipt.created);
    let written = store.content(OVERLAY).unwrap();
    assert!(trust.is_approved(OVERLAY, written.as_bytes()));

    let receipt = use_case(store.clone(), trust.clone())
        .execute(patch(
            ConfigLayer::Overlay,
            OVERLAY,
            "agents.defaults.effort",
            json!("low"),
        ))
        .unwrap();
    assert!(!receipt.created, "a trusted overlay is patched in place");
    assert!(trust.is_approved(OVERLAY, store.content(OVERLAY).unwrap().as_bytes()));
    assert_eq!(
        document(&store, OVERLAY),
        json!({"agents":{"defaults":{"model":"m","effort":"low"}}})
    );
}

#[test]
fn an_untrusted_overlay_is_not_patched() {
    let content = r#"{"agents":{"defaults":{"model":"theirs"}}}"#;
    let store = MemoryStore::with(&[(OVERLAY, content)]);
    let error = use_case(store.clone(), Arc::new(FakeTrust::default()))
        .execute(patch(
            ConfigLayer::Overlay,
            OVERLAY,
            "agents.defaults.effort",
            json!("low"),
        ))
        .unwrap_err();
    assert_eq!(
        error,
        ConfigPatchError::UntrustedOverlay {
            path: PathBuf::from(OVERLAY),
            fingerprint: "fp-42".into()
        }
    );
    assert!(error.to_string().contains("quecto config trust"));
    assert_eq!(store.content(OVERLAY).unwrap(), content);
}

#[test]
fn a_failed_trust_record_after_an_overlay_write_is_reported() {
    let store = MemoryStore::with(&[]);
    let trust = Arc::new(FakeTrust {
        fail_approve: true,
        ..Default::default()
    });
    let error = use_case(store, trust)
        .execute(patch(
            ConfigLayer::Overlay,
            OVERLAY,
            "agents.defaults.model",
            json!("m"),
        ))
        .unwrap_err();
    assert_eq!(
        error,
        ConfigPatchError::Trust {
            path: PathBuf::from(OVERLAY),
            reason: "store unwritable".into()
        }
    );
    assert!(error.to_string().contains("could not record it as trusted"));
}

#[test]
fn debug_and_display_are_informative() {
    assert!(
        format!(
            "{:?}",
            use_case(MemoryStore::with(&[]), Arc::new(FakeTrust::default()))
        )
        .contains("PatchConfiguration")
    );
    assert!(
        ConfigPatchError::InvalidKeyPath("x".into())
            .to_string()
            .contains("agents.defaults.model")
    );
}
