use super::*;
use crate::application::configuration::dto::ConfigLayers;
use crate::application::configuration::use_cases::fakes::{FakeTrust, FakeValidator, MemoryStore};
use serde_json::json;
use std::path::PathBuf;

const GLOBAL: &str = "/home/u/.quecto/config.json";
const OVERLAY: &str = "/work/.quecto/config.json";
const LEGACY: &str = "/work/config.json";

fn layered() -> ConfigSelection {
    ConfigSelection::Layered(ConfigLayers {
        global: PathBuf::from(GLOBAL),
        overlay: Some(PathBuf::from(OVERLAY)),
        legacy_local: Some(PathBuf::from(LEGACY)),
    })
}

fn use_case(
    store: Arc<MemoryStore>,
    trust: Arc<FakeTrust>,
) -> (ResolveEffectiveConfig, Arc<FakeValidator>) {
    let validator = Arc::new(FakeValidator::default());
    (
        ResolveEffectiveConfig::new(store, validator.clone(), trust),
        validator,
    )
}

#[test]
fn a_trusted_overlay_merges_over_the_global_file() {
    let overlay = r#"{"agents":{"defaults":{"model":"local"}}}"#;
    let store = MemoryStore::with(&[
        (
            GLOBAL,
            r#"{"agents":{"defaults":{"model":"global","effort":"high"}},"providers":{"openai":{"api_key":"k"}}}"#,
        ),
        (OVERLAY, overlay),
    ]);
    let (resolve, validator) = use_case(store, FakeTrust::trusting(OVERLAY, overlay));
    let effective = resolve.execute(&layered()).unwrap();
    assert_eq!(
        effective.document,
        json!({"agents":{"defaults":{"model":"local","effort":"high"}},"providers":{"openai":{"api_key":"k"}}})
    );
    assert_eq!(
        effective.sources,
        ConfigSources {
            base: PathBuf::from(GLOBAL),
            explicit: false,
            overlay: Some(OverlayReport {
                path: PathBuf::from(OVERLAY),
                state: OverlayState::Applied
            }),
            legacy_local: None,
        }
    );
    assert_eq!(
        effective.sources.applied_overlay(),
        Some(&PathBuf::from(OVERLAY))
    );
    let validated = validator.validated.lock().unwrap();
    assert_eq!(
        validated.len(),
        3,
        "the global file alone, the overlay as a layer, then the merge"
    );
    assert_eq!(
        validated[1],
        json!({"agents":{"defaults":{"model":"local"}}})
    );
}

#[test]
fn an_untrusted_overlay_is_reported_and_not_applied() {
    let store = MemoryStore::with(&[
        (GLOBAL, r#"{"agents":{"defaults":{"model":"global"}}}"#),
        (OVERLAY, r#"{"agents":{"defaults":{"model":"local"}}}"#),
    ]);
    let (resolve, _) = use_case(store, Arc::new(FakeTrust::default()));
    let effective = resolve.execute(&layered()).unwrap();
    assert_eq!(
        effective.document,
        json!({"agents":{"defaults":{"model":"global"}}})
    );
    assert_eq!(
        effective.sources.overlay,
        Some(OverlayReport {
            path: PathBuf::from(OVERLAY),
            state: OverlayState::Untrusted {
                fingerprint: "fp-41".into()
            }
        })
    );
    assert_eq!(effective.sources.applied_overlay(), None);
}

#[test]
fn an_untrusted_overlay_is_not_even_parsed() {
    let store = MemoryStore::with(&[(GLOBAL, "{}"), (OVERLAY, "{ not json")]);
    let (resolve, _) = use_case(store, Arc::new(FakeTrust::default()));
    let effective = resolve.execute(&layered()).unwrap();
    assert!(matches!(
        effective.sources.overlay.unwrap().state,
        OverlayState::Untrusted { .. }
    ));
}

#[test]
fn an_absent_overlay_and_an_absent_global_file_yield_an_empty_document() {
    let store = MemoryStore::with(&[]);
    let (resolve, _) = use_case(store, Arc::new(FakeTrust::default()));
    let effective = resolve.execute(&layered()).unwrap();
    assert_eq!(effective.document, json!({}));
    assert_eq!(
        effective.sources.overlay,
        Some(OverlayReport {
            path: PathBuf::from(OVERLAY),
            state: OverlayState::Absent
        })
    );
}

#[test]
fn a_trusted_overlay_with_a_global_only_key_is_refused_naming_the_key() {
    for (content, key) in [
        (r#"{"providers":{"openai":{"api_base":"x"}}}"#, "providers"),
        (r#"{"admission":null}"#, "admission"),
    ] {
        let store = MemoryStore::with(&[(GLOBAL, "{}"), (OVERLAY, content)]);
        let (resolve, _) = use_case(store, FakeTrust::trusting(OVERLAY, content));
        let error = resolve.execute(&layered()).unwrap_err();
        assert_eq!(
            error,
            EffectiveConfigError::GlobalOnlyKey {
                path: PathBuf::from(OVERLAY),
                key: key.into()
            }
        );
        let text = error.to_string();
        assert!(
            text.contains(OVERLAY) && text.contains(&format!("`{key}` is global-only")),
            "{text}"
        );
    }
}

#[test]
fn a_trusted_but_broken_overlay_is_an_error_naming_the_file() {
    let content = "{ not json";
    let store = MemoryStore::with(&[(GLOBAL, "{}"), (OVERLAY, content)]);
    let (resolve, _) = use_case(store, FakeTrust::trusting(OVERLAY, content));
    let error = resolve.execute(&layered()).unwrap_err();
    assert!(
        matches!(&error, EffectiveConfigError::Parse { path, .. } if path == Path::new(OVERLAY))
    );
    assert!(
        error
            .to_string()
            .starts_with("failed to load config /work/.quecto/config.json")
    );

    let content = "[1]";
    let store = MemoryStore::with(&[(GLOBAL, "{}"), (OVERLAY, content)]);
    let (resolve, _) = use_case(store, FakeTrust::trusting(OVERLAY, content));
    assert_eq!(
        resolve.execute(&layered()).unwrap_err(),
        EffectiveConfigError::NotAnObject(PathBuf::from(OVERLAY))
    );

    let content = r#"{"agents":{"defaults":{"effort":"bogus"}}}"#;
    let store = MemoryStore::with(&[(GLOBAL, "{}"), (OVERLAY, content)]);
    let (resolve, _) = use_case(store, FakeTrust::trusting(OVERLAY, content));
    assert_eq!(
        resolve.execute(&layered()).unwrap_err(),
        EffectiveConfigError::Invalid {
            path: PathBuf::from(OVERLAY),
            reason: "invalid effort level 'bogus'".into()
        }
    );

    let content = r#"{"container_scripts":{}}"#;
    let store = MemoryStore::with(&[(GLOBAL, "{}"), (OVERLAY, content)]);
    let (resolve, _) = use_case(store, FakeTrust::trusting(OVERLAY, content));
    assert!(matches!(
        resolve.execute(&layered()).unwrap_err(),
        EffectiveConfigError::Invalid { path, .. } if path == Path::new(OVERLAY)
    ));
}

#[test]
fn references_resolve_against_each_layers_own_file() {
    let overlay = r#"{"workflow":{"marker":"?"}}"#;
    let store = MemoryStore::with(&[
        (
            GLOBAL,
            r#"{"workflow":{"marker":"?","auto_continue":true}}"#,
        ),
        (OVERLAY, overlay),
    ]);
    let (resolve, _) = use_case(store, FakeTrust::trusting(OVERLAY, overlay));
    let effective = resolve.execute(&layered()).unwrap();
    assert_eq!(
        effective.document["workflow"],
        json!({"marker": OVERLAY, "auto_continue": true})
    );
}

#[test]
fn global_file_failures_are_errors_naming_the_global_file() {
    let store = MemoryStore::with(&[(GLOBAL, "{ nope")]);
    let (resolve, _) = use_case(store, Arc::new(FakeTrust::default()));
    assert!(matches!(
        resolve.execute(&layered()).unwrap_err(),
        EffectiveConfigError::Parse { path, .. } if path == Path::new(GLOBAL)
    ));

    let mut store = MemoryStore::default();
    store.broken.insert(PathBuf::from(GLOBAL));
    let (resolve, _) = use_case(Arc::new(store), Arc::new(FakeTrust::default()));
    let error = resolve.execute(&layered()).unwrap_err();
    assert_eq!(
        error,
        EffectiveConfigError::Read {
            path: PathBuf::from(GLOBAL),
            reason: "permission denied".into()
        }
    );
    assert!(error.to_string().contains("permission denied"));

    let store = MemoryStore::with(&[(GLOBAL, r#"{"agents":{"defaults":{"effort":"bogus"}}}"#)]);
    let (resolve, _) = use_case(store, Arc::new(FakeTrust::default()));
    assert!(matches!(
        resolve.execute(&layered()).unwrap_err(),
        EffectiveConfigError::Invalid { path, .. } if path == Path::new(GLOBAL)
    ));
}

#[test]
fn a_present_unreadable_overlay_is_an_error_not_a_fallback() {
    let mut store = MemoryStore::default();
    store.broken.insert(PathBuf::from(OVERLAY));
    let (resolve, _) = use_case(Arc::new(store), Arc::new(FakeTrust::default()));
    assert!(matches!(
        resolve.execute(&layered()).unwrap_err(),
        EffectiveConfigError::Read { path, .. } if path == Path::new(OVERLAY)
    ));
}

#[test]
fn the_retired_local_file_is_reported_when_present_and_never_loaded() {
    let store = MemoryStore::with(&[
        (GLOBAL, r#"{"agents":{"defaults":{"model":"global"}}}"#),
        (LEGACY, r#"{"agents":{"defaults":{"model":"legacy"}}}"#),
    ]);
    let (resolve, _) = use_case(store, Arc::new(FakeTrust::default()));
    let effective = resolve.execute(&layered()).unwrap();
    assert_eq!(effective.document["agents"]["defaults"]["model"], "global");
    assert_eq!(effective.sources.legacy_local, Some(PathBuf::from(LEGACY)));
}

#[test]
fn without_an_overlay_location_only_the_global_file_loads() {
    let store = MemoryStore::with(&[(GLOBAL, r#"{"agents":{}}"#)]);
    let (resolve, _) = use_case(store, Arc::new(FakeTrust::default()));
    let selection = ConfigSelection::Layered(ConfigLayers {
        global: PathBuf::from(GLOBAL),
        overlay: None,
        legacy_local: None,
    });
    let effective = resolve.execute(&selection).unwrap();
    assert_eq!(effective.sources.overlay, None);
    assert_eq!(effective.sources.legacy_local, None);
}

#[test]
fn an_explicit_file_replaces_both_layers_and_must_exist() {
    let store = MemoryStore::with(&[
        (GLOBAL, r#"{"agents":{"defaults":{"model":"global"}}}"#),
        (
            "/x/c.json",
            r#"{"agents":{"defaults":{"model":"explicit"}},"admission":null}"#,
        ),
    ]);
    let (resolve, _) = use_case(store, Arc::new(FakeTrust::default()));
    let effective = resolve
        .execute(&ConfigSelection::Explicit(PathBuf::from("/x/c.json")))
        .unwrap();
    assert_eq!(
        effective.document["agents"]["defaults"]["model"],
        "explicit"
    );
    assert_eq!(
        effective.sources,
        ConfigSources {
            base: PathBuf::from("/x/c.json"),
            explicit: true,
            overlay: None,
            legacy_local: None
        }
    );
    let error = resolve
        .execute(&ConfigSelection::Explicit(PathBuf::from("/x/missing.json")))
        .unwrap_err();
    assert_eq!(
        error,
        EffectiveConfigError::Missing(PathBuf::from("/x/missing.json"))
    );
    assert_eq!(error.to_string(), "config not found: /x/missing.json");
}

#[test]
fn error_display_names_the_file_for_every_variant() {
    let path = PathBuf::from("/p/config.json");
    for error in [
        EffectiveConfigError::Read {
            path: path.clone(),
            reason: "r".into(),
        },
        EffectiveConfigError::Parse {
            path: path.clone(),
            reason: "r".into(),
        },
        EffectiveConfigError::NotAnObject(path.clone()),
        EffectiveConfigError::GlobalOnlyKey {
            path: path.clone(),
            key: "k".into(),
        },
        EffectiveConfigError::Invalid {
            path: path.clone(),
            reason: "r".into(),
        },
    ] {
        let text = error.to_string();
        assert!(
            text.starts_with("failed to load config /p/config.json"),
            "{text}"
        );
    }
    assert!(
        format!(
            "{:?}",
            ResolveEffectiveConfig::new(
                MemoryStore::with(&[]),
                Arc::new(FakeValidator::default()),
                Arc::new(FakeTrust::default())
            )
        )
        .contains("ResolveEffectiveConfig")
    );
}

#[test]
fn consent_at_the_prompt_applies_and_records_the_overlay_after_the_same_checks() {
    let overlay = r#"{"agents":{"defaults":{"model":"local"}}}"#;
    let store = MemoryStore::with(&[(GLOBAL, "{}"), (OVERLAY, overlay)]);
    let trust = Arc::new(FakeTrust {
        consents: true,
        ..Default::default()
    });
    let (resolve, _) = use_case(store, trust.clone());
    let effective = resolve.execute(&layered()).unwrap();
    assert_eq!(effective.document["agents"]["defaults"]["model"], "local");
    assert!(
        trust.is_approved(OVERLAY, overlay.as_bytes()),
        "consent is recorded"
    );
    assert_eq!(trust.offered.lock().unwrap().len(), 1);

    // An overlay that would not apply is never offered, so a "y" cannot
    // record it.
    for content in [
        r#"{"providers":{}}"#,
        r#"{"agents":{"defaults":{"effort":"bogus"}}}"#,
        "{ nope",
    ] {
        let store = MemoryStore::with(&[(GLOBAL, "{}"), (OVERLAY, content)]);
        let trust = Arc::new(FakeTrust {
            consents: true,
            ..Default::default()
        });
        let (resolve, _) = use_case(store, trust.clone());
        // Never offered: reported as untrusted, the run stays on the global file.
        let effective = resolve.execute(&layered()).unwrap();
        assert!(
            matches!(
                effective.sources.overlay.unwrap().state,
                OverlayState::Untrusted { .. }
            ),
            "{content}"
        );
        assert!(trust.offered.lock().unwrap().is_empty(), "{content}");
        assert!(!trust.is_approved(OVERLAY, content.as_bytes()), "{content}");
    }

    let store = MemoryStore::with(&[(GLOBAL, "{}"), (OVERLAY, overlay)]);
    let trust = Arc::new(FakeTrust {
        consents: true,
        fail_approve: true,
        ..Default::default()
    });
    let (resolve, _) = use_case(store, trust);
    let error = resolve.execute(&layered()).unwrap_err();
    assert!(
        error.to_string().contains("could not record trust"),
        "{error}"
    );
}

#[test]
fn an_overlay_may_add_a_non_default_container_config_and_an_invalid_merge_names_both_files() {
    let overlay = r#"{"container_configs":{"h":{"create":["b"]}}}"#;
    let store = MemoryStore::with(&[
        (
            GLOBAL,
            r#"{"container_configs":{"g":{"default":true,"create":["a"]}}}"#,
        ),
        (OVERLAY, overlay),
    ]);
    let (resolve, _) = use_case(store, FakeTrust::trusting(OVERLAY, overlay));
    let effective = resolve.execute(&layered()).unwrap();
    assert_eq!(
        effective.document["container_configs"]["h"]["create"],
        json!(["b"])
    );
    assert_eq!(
        effective.document["container_configs"]["g"]["default"],
        true
    );

    // Replacing the only default entry with a non-default one leaves the
    // merge without a default: each layer is fine, the merge is not.
    let overlay = r#"{"container_configs":{"g":{"exec":["x"]}}}"#;
    let store = MemoryStore::with(&[
        (
            GLOBAL,
            r#"{"container_configs":{"g":{"default":true,"create":["a"]}}}"#,
        ),
        (OVERLAY, overlay),
    ]);
    let (resolve, _) = use_case(store, FakeTrust::trusting(OVERLAY, overlay));
    let error = resolve.execute(&layered()).unwrap_err();
    assert!(
        matches!(&error, EffectiveConfigError::InvalidMerge { global, overlay, .. }
        if global.as_deref() == Some(Path::new(GLOBAL)) && overlay == Path::new(OVERLAY))
    );
    let text = error.to_string();
    assert!(
        text.contains(OVERLAY) && text.contains(GLOBAL) && text.contains("merged over"),
        "{text}"
    );

    // Without an overlay the global file is validated in full and named.
    let store = MemoryStore::with(&[(GLOBAL, r#"{"container_configs":{"g":{"create":["a"]}}}"#)]);
    let (resolve, _) = use_case(store, Arc::new(FakeTrust::default()));
    assert!(matches!(resolve.execute(&layered()).unwrap_err(),
        EffectiveConfigError::Invalid { path, .. } if path == Path::new(GLOBAL)));
}

#[test]
fn an_unrelated_config_json_in_the_working_directory_is_not_a_retired_config() {
    for content in [r#"{"name":"my-app","version":1}"#, "[1,2]", "not json"] {
        let store = MemoryStore::with(&[(GLOBAL, "{}"), (LEGACY, content)]);
        let (resolve, _) = use_case(store, Arc::new(FakeTrust::default()));
        assert_eq!(
            resolve.execute(&layered()).unwrap().sources.legacy_local,
            None,
            "{content}"
        );
    }
    let store = MemoryStore::with(&[(GLOBAL, "{}"), (LEGACY, r#"{"tools":{}}"#)]);
    let (resolve, _) = use_case(store, Arc::new(FakeTrust::default()));
    assert_eq!(
        resolve.execute(&layered()).unwrap().sources.legacy_local,
        Some(PathBuf::from(LEGACY))
    );
}

#[test]
fn the_global_file_must_be_valid_on_its_own_even_when_the_overlay_would_repair_it() {
    let overlay = r#"{"container_configs":{"l":{"default":true,"create":["c"]}}}"#;
    let store = MemoryStore::with(&[
        (
            GLOBAL,
            r#"{"container_configs":{"a":{"default":true},"b":{"default":true}}}"#,
        ),
        (OVERLAY, overlay),
    ]);
    let (resolve, _) = use_case(store, FakeTrust::trusting(OVERLAY, overlay));
    assert!(matches!(
        resolve.execute(&layered()).unwrap_err(),
        EffectiveConfigError::Invalid { path, .. } if path == Path::new(GLOBAL)
    ));
}

#[test]
fn a_prompt_is_offered_only_for_an_overlay_that_would_apply_and_an_absent_global_is_named_as_such()
{
    let content = r#"{"providers":{}}"#;
    let store = MemoryStore::with(&[(GLOBAL, "{}"), (OVERLAY, content)]);
    let trust = Arc::new(FakeTrust {
        consents: true,
        ..Default::default()
    });
    let (resolve, _) = use_case(store, trust.clone());
    let effective = resolve.execute(&layered()).unwrap();
    assert!(matches!(
        effective.sources.overlay.unwrap().state,
        OverlayState::Untrusted { .. }
    ));
    assert!(
        trust.offered.lock().unwrap().is_empty(),
        "nobody is asked to trust a refused overlay"
    );

    let overlay = r#"{"container_configs":{"h":{"create":["b"]}}}"#;
    let store = MemoryStore::with(&[(OVERLAY, overlay)]);
    let (resolve, _) = use_case(store, FakeTrust::trusting(OVERLAY, overlay));
    let error = resolve.execute(&layered()).unwrap_err();
    assert!(matches!(
        &error,
        EffectiveConfigError::InvalidMerge { global: None, .. }
    ));
    assert!(error.to_string().contains("no global file"), "{error}");
}
