use super::*;
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

/// A patch of `layer` in the layered selection; `path` documents which
/// file the layer addresses and is asserted against the selection.
fn patch(layer: ConfigLayer, path: &str, key_path: &str, value: Value) -> ConfigPatch {
    let selection = layered();
    let expected = match layer {
        ConfigLayer::Global => selection.path(),
        ConfigLayer::Overlay => selection.overlay_path().unwrap(),
    };
    assert_eq!(expected, Path::new(path));
    ConfigPatch {
        selection,
        layer,
        key_path: key_path.into(),
        value,
    }
}

fn use_case(store: Arc<MemoryStore>, trust: Arc<FakeTrust>) -> PatchConfiguration {
    let validator = Arc::new(FakeValidator::default());
    let resolve = Arc::new(ResolveEffectiveConfig::new(
        store.clone(),
        validator.clone(),
        trust.clone(),
    ));
    PatchConfiguration::new(store.clone(), store, validator, trust, resolve)
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
        self::use_case(store, Arc::new(FakeTrust::default()))
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
fn an_overlay_patch_creates_the_file_and_records_trust_for_the_bytes_the_writer_returned() {
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
    // The fake writer returns a rendering that differs from what its store
    // then serves: the approval must be for the returned bytes, never for
    // a read-back (a racing writer could have replaced the file by then).
    let returned = store.returned.lock().unwrap().last().unwrap().clone();
    let on_disk = store.content(OVERLAY).unwrap();
    assert_ne!(returned, on_disk.as_bytes());
    assert!(trust.is_approved(OVERLAY, &returned), "the bytes written");
    assert!(
        !trust.is_approved(OVERLAY, on_disk.as_bytes()),
        "not whatever the store serves afterwards"
    );

    // The second patch finds the on-disk content: the fake's store serves
    // compact bytes, so trust it explicitly (as a real writer's read-back
    // would match) before patching in place.
    let trust = FakeTrust::trusting(OVERLAY, &on_disk);
    let receipt = use_case(store.clone(), trust.clone())
        .execute(patch(
            ConfigLayer::Overlay,
            OVERLAY,
            "agents.defaults.effort",
            json!("low"),
        ))
        .unwrap();
    assert!(!receipt.created, "a trusted overlay is patched in place");
    let returned = store.returned.lock().unwrap().last().unwrap().clone();
    assert!(trust.is_approved(OVERLAY, &returned));
    assert_eq!(
        document(&store, OVERLAY),
        json!({"agents":{"defaults":{"model":"m","effort":"low"}}})
    );
}

#[test]
fn an_overlay_patch_that_bricks_the_merge_is_refused_before_anything_is_written() {
    // Valid as a layer (a non-default container config), invalid as the
    // effective configuration: no container config is the default.
    let store = MemoryStore::with(&[(GLOBAL, r#"{"agents":{"defaults":{"model":"g"}}}"#)]);
    let error = use_case(store.clone(), Arc::new(FakeTrust::default()))
        .execute(patch(
            ConfigLayer::Overlay,
            OVERLAY,
            "container_configs.app.create",
            json!(["x"]),
        ))
        .unwrap_err();
    let ConfigPatchError::InvalidMerge { path, reason } = &error else {
        panic!("expected InvalidMerge, got {error:?}");
    };
    assert_eq!(path, Path::new(OVERLAY));
    assert!(
        matches!(reason, EffectiveConfigError::InvalidMerge { .. }),
        "{reason:?}"
    );
    let message = error.to_string();
    assert!(message.contains("refusing to write"), "{message}");
    assert!(
        message.contains(OVERLAY) && message.contains(GLOBAL),
        "{message}"
    );
    assert!(
        message.contains("no container config is labeled"),
        "{message}"
    );
    assert_eq!(store.content(OVERLAY), None, "nothing written");
    assert!(store.returned.lock().unwrap().is_empty());
}

#[test]
fn a_global_patch_is_validated_against_the_trusted_overlay_it_merges_with() {
    let overlay = r#"{"container_configs":{"app":{"create":["x"]}}}"#;
    let store = MemoryStore::with(&[(GLOBAL, r#"{}"#), (OVERLAY, overlay)]);
    let trust = FakeTrust::trusting(OVERLAY, overlay);
    let validator = Arc::new(FakeValidator::default());
    let resolve = Arc::new(ResolveEffectiveConfig::new(
        store.clone(),
        validator.clone(),
        trust.clone(),
    ));
    PatchConfiguration::new(
        store.clone(),
        store.clone(),
        validator.clone(),
        trust,
        resolve,
    )
    .execute(patch(
        ConfigLayer::Global,
        GLOBAL,
        "container_configs.shared",
        json!({"default": true, "create": ["y"]}),
    ))
    .unwrap();
    let validated = validator.validated.lock().unwrap();
    assert!(
        validated.iter().any(|document| {
            document.pointer("/container_configs/app").is_some()
                && document.pointer("/container_configs/shared").is_some()
        }),
        "the merge with the trusted overlay was validated before the write: {validated:?}"
    );
    assert_eq!(
        document(&store, GLOBAL),
        json!({"container_configs":{"shared":{"default":true,"create":["y"]}}}),
        "the overlay's entry is not written into the global file"
    );
}

#[test]
fn an_overlay_patch_refuses_a_symbolic_link_and_a_selection_without_an_overlay() {
    let mut store = MemoryStore::default();
    store.symlinks.insert(PathBuf::from(OVERLAY));
    let store = Arc::new(store);
    let error = use_case(store.clone(), Arc::new(FakeTrust::default()))
        .execute(patch(
            ConfigLayer::Overlay,
            OVERLAY,
            "agents.defaults.model",
            json!("m"),
        ))
        .unwrap_err();
    assert!(
        matches!(&error, ConfigPatchError::Refused { path, .. } if path == Path::new(OVERLAY)),
        "{error:?}"
    );
    assert!(error.to_string().contains("symbolic link"), "{error}");
    assert_eq!(store.content(OVERLAY), None);

    // The `.quecto` directory as a link: refused too, and the file behind
    // it is neither read (its trust is irrelevant) nor written.
    let mut store = MemoryStore::default();
    store
        .symlinks
        .insert(Path::new(OVERLAY).parent().unwrap().to_path_buf());
    store
        .files
        .lock()
        .unwrap()
        .insert(PathBuf::from(OVERLAY), b"{}".to_vec());
    let store = Arc::new(store);
    let error = use_case(store.clone(), FakeTrust::trusting(OVERLAY, "{}"))
        .execute(patch(
            ConfigLayer::Overlay,
            OVERLAY,
            "agents.defaults.model",
            json!("m"),
        ))
        .unwrap_err();
    assert!(
        matches!(error, ConfigPatchError::Refused { .. }),
        "{error:?}"
    );
    assert_eq!(store.content(OVERLAY).as_deref(), Some("{}"));

    let error = use_case(MemoryStore::with(&[]), Arc::new(FakeTrust::default()))
        .execute(ConfigPatch {
            selection: ConfigSelection::Explicit(PathBuf::from(GLOBAL)),
            layer: ConfigLayer::Overlay,
            key_path: "agents.defaults.model".into(),
            value: json!("m"),
        })
        .unwrap_err();
    assert_eq!(error, ConfigPatchError::NoOverlayLocation);
    assert!(error.to_string().contains("no repo-local overlay"));
}

#[test]
fn an_explicit_selection_is_patched_as_the_global_layer() {
    let store = MemoryStore::with(&[("/x/c.json", r#"{"agents":{"defaults":{"model":"e"}}}"#)]);
    use_case(store.clone(), Arc::new(FakeTrust::default()))
        .execute(ConfigPatch {
            selection: ConfigSelection::Explicit(PathBuf::from("/x/c.json")),
            layer: ConfigLayer::Global,
            key_path: "agents.defaults.model".into(),
            value: json!("f"),
        })
        .unwrap();
    assert_eq!(
        document(&store, "/x/c.json"),
        json!({"agents":{"defaults":{"model":"f"}}})
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

#[test]
fn an_overlay_patch_may_add_a_non_default_container_config() {
    // Valid as a layer, and valid merged: the global file has the default.
    let store = MemoryStore::with(&[(
        GLOBAL,
        r#"{"container_configs":{"shared":{"default":true,"create":["a"]}}}"#,
    )]);
    use_case(store.clone(), Arc::new(FakeTrust::default()))
        .execute(patch(
            ConfigLayer::Overlay,
            OVERLAY,
            "container_configs.h",
            json!({"create":["b"]}),
        ))
        .unwrap();
    assert_eq!(
        document(&store, OVERLAY),
        json!({"container_configs":{"h":{"create":["b"]}}})
    );
    let error = use_case(store, Arc::new(FakeTrust::default()))
        .execute(patch(
            ConfigLayer::Global,
            GLOBAL,
            "container_configs.shared.default",
            json!(false),
        ))
        .unwrap_err();
    assert!(
        matches!(error, ConfigPatchError::Invalid { .. }),
        "the global file is complete on its own: {error}"
    );
    let root = ConfigPatchError::NotAnObject {
        path: PathBuf::from(GLOBAL),
        at: String::new(),
    };
    assert!(
        root.to_string()
            .contains("the document is not a JSON object")
    );
}

// ── unset (#2024 S2) ────────────────────────────────────────────────────────

fn unset(layer: ConfigLayer, key_path: &str) -> ConfigUnset {
    ConfigUnset {
        selection: layered(),
        layer,
        key_path: key_path.into(),
    }
}

#[test]
fn unset_removes_only_the_addressed_key_and_re_records_overlay_trust() {
    let content = r#"{"agents":{"defaults":{"model":"local","effort":"high"}},"zeta":"kept"}"#;
    let store = MemoryStore::with(&[(OVERLAY, content)]);
    let trust = FakeTrust::trusting(OVERLAY, content);
    let receipt = use_case(store.clone(), trust.clone())
        .unset(unset(ConfigLayer::Overlay, "agents.defaults.model"))
        .unwrap();
    assert_eq!(receipt.path, PathBuf::from(OVERLAY));
    assert!(!receipt.created);
    assert_eq!(
        document(&store, OVERLAY),
        json!({"agents":{"defaults":{"effort":"high"}},"zeta":"kept"})
    );
    let returned = store.returned.lock().unwrap().last().unwrap().clone();
    assert!(
        trust.is_approved(OVERLAY, &returned),
        "trust re-recorded for the bytes written"
    );
    assert!(!trust.is_approved(OVERLAY, content.as_bytes()));
}

#[test]
fn unset_of_a_key_the_layer_does_not_set_is_an_error_and_writes_nothing() {
    let content = r#"{"agents":{"defaults":{"model":"global"}}}"#;
    let store = MemoryStore::with(&[(GLOBAL, content)]);
    let use_case = use_case(store.clone(), Arc::new(FakeTrust::default()));
    let error = use_case
        .unset(unset(ConfigLayer::Global, "agents.defaults.effort"))
        .unwrap_err();
    assert_eq!(
        error,
        ConfigPatchError::NotSet {
            path: PathBuf::from(GLOBAL),
            key_path: "agents.defaults.effort".into(),
        }
    );
    let message = error.to_string();
    assert!(message.contains("agents.defaults.effort"), "{message}");
    assert!(message.contains("not set"), "{message}");
    assert_eq!(store.content(GLOBAL).unwrap(), content);
    // A missing overlay sets nothing either.
    let error = use_case
        .unset(unset(ConfigLayer::Overlay, "agents.defaults.model"))
        .unwrap_err();
    assert!(matches!(error, ConfigPatchError::NotSet { .. }), "{error}");
    assert!(store.content(OVERLAY).is_none());
}

#[test]
fn unset_shares_the_patch_refusals() {
    let store = MemoryStore::with(&[(OVERLAY, r#"{"agents":{"defaults":{"model":"theirs"}}}"#)]);
    let use_case = use_case(store.clone(), Arc::new(FakeTrust::default()));
    assert!(matches!(
        use_case
            .unset(unset(ConfigLayer::Overlay, "agents.defaults.model"))
            .unwrap_err(),
        ConfigPatchError::UntrustedOverlay { .. }
    ));
    let global_only = use_case
        .unset(unset(ConfigLayer::Overlay, "providers.openai"))
        .unwrap_err();
    assert!(matches!(
        global_only,
        ConfigPatchError::GlobalOnlyKey { .. }
    ));
    assert!(
        global_only
            .to_string()
            .starts_with("cannot change `providers`"),
        "{global_only}"
    );
    assert!(matches!(
        use_case
            .unset(unset(ConfigLayer::Global, "a..b"))
            .unwrap_err(),
        ConfigPatchError::InvalidKeyPath(_)
    ));
}
