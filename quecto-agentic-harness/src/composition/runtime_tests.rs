//! Tests for the runtime-composition wiring (issue #1573, #1849 PR 1): the real
//! factory, sources, and stores behind `compose_and_publish_runtime`, and the
//! selection verdict the change-active-model plan reads over what it
//! published (#1847), driven with on-disk models.json fixtures (no network).

use super::*;
use crate::domain::catalogue::UnavailableReason;

fn write_models_json(dir: &Path, body: &str) {
    std::fs::write(dir.join("models.json"), body).unwrap();
}

const RUNNABLE_AND_KEYLESS: &str = r#"{"providers":{
    "wired":{"api":"openai-completions","apiKey":"sk-wired","baseUrl":"https://api.example.test/v1",
        "models":[{"id":"wired-model","name":"Wired Model"}]},
    "keyless":{"api":"openai-completions",
        "models":[{"id":"keyless-model","name":"Keyless Model"}]}
}}"#;

fn compose(dir: &Path) -> Result<Arc<CatalogueRuntimeSnapshot>, RuntimeCompositionError> {
    compose_and_publish_runtime(&Config::default(), dir, &reqwest::Client::new())
}

/// The selection verdict the change-active-model plan carries (#1847),
/// read over the published runtime generation of `dir`.
fn select_model(
    dir: &Path,
    model: &str,
) -> crate::application::catalogue::dto::ModelSelectionVerdict {
    crate::composition::catalogue::build_catalogue_handles(dir, None)
        .model
        .plan(model)
        .verdict
}

#[test]
fn failed_composition_publishes_no_runtime_and_selection_reports_no_runtime() {
    let tmp = tempfile::tempdir().unwrap();
    // No credentials anywhere: the factory fails, nothing is published.
    let error = compose(tmp.path()).expect_err("composition fails without providers");
    assert!(error.error.contains("no LLM providers configured"));
    assert!(error.retained.is_none());
    assert!(runtime_store_for(tmp.path()).current().is_none());
    assert_eq!(
        select_model(tmp.path(), "openai-api/gpt-5"),
        crate::application::catalogue::dto::ModelSelectionVerdict::NoRuntime
    );
}

#[test]
fn composed_runtime_and_catalogue_share_one_generation_and_selection_matches() {
    let tmp = tempfile::tempdir().unwrap();
    write_models_json(tmp.path(), RUNNABLE_AND_KEYLESS);
    let snapshot = compose(tmp.path()).expect("composition succeeds");
    assert_eq!(
        snapshot.generation(),
        snapshot_store_for(tmp.path()).current().generation(),
        "runtime and catalogue stores publish the same generation"
    );
    match select_model(tmp.path(), "wired/wired-model") {
        crate::application::catalogue::dto::ModelSelectionVerdict::Runnable {
            provider,
            generation,
        } => {
            assert_eq!(provider, "wired");
            assert_eq!(generation, snapshot.generation());
        }
        other => panic!("runnable model selects, got {other:?}"),
    }

    // A failed re-composition retains this generation.
    std::fs::remove_file(tmp.path().join("models.json")).unwrap();
    let error = compose(tmp.path()).expect_err("re-composition fails");
    let retained = error.retained.expect("previous runtime retained");
    assert_eq!(retained.generation(), snapshot.generation());
    assert_eq!(
        runtime_store_for(tmp.path())
            .current()
            .expect("still published")
            .generation(),
        snapshot.generation()
    );
}

#[test]
fn selection_returns_structured_reasons_for_unknown_and_unrunnable_models() {
    let tmp = tempfile::tempdir().unwrap();
    write_models_json(tmp.path(), RUNNABLE_AND_KEYLESS);
    compose(tmp.path()).expect("composition succeeds");
    use crate::application::catalogue::dto::ModelSelectionVerdict;
    assert_eq!(
        select_model(tmp.path(), "wired/no-such-model"),
        ModelSelectionVerdict::Unknown {
            reference: "wired/no-such-model".to_string()
        }
    );
    assert_eq!(
        select_model(tmp.path(), "not a qualified ref"),
        ModelSelectionVerdict::Unknown {
            reference: "not a qualified ref".to_string()
        }
    );
    match select_model(tmp.path(), "keyless/keyless-model") {
        ModelSelectionVerdict::NotRunnable { reasons, .. } => {
            assert!(reasons.contains(&UnavailableReason::MissingCredential));
        }
        other => panic!("expected NotRunnable, got {other:?}"),
    }
}

#[test]
fn admission_candidate_is_the_configured_proposal_or_none() {
    let mut config = Config::default();
    assert!(admission_candidate(&config).is_none());
    config.admission = serde_json::from_str(
        r#"{"directory":"/tmp/x","groups":{"g":{"capacity":1,"reserve":0,"min_interval_ms":1,"queue_capacity":1,"queue_timeout_ms":1,"attempt_timeout_ms":1,"fallback_base_ms":1,"max_cooldown_ms":1}},"aliases":{"a":"g"},"bindings":{"openai":"a"}}"#,
    )
    .unwrap();
    assert_eq!(
        admission_candidate(&config).unwrap().bindings["openai"],
        "a"
    );
    config.admission.as_mut().unwrap().bindings.clear();
    assert!(
        admission_candidate(&config).is_none(),
        "invalid sections are not candidates"
    );
}

/// M5c (#2024 S3): only a child composes with `inherit = true`; a root offers
/// its configured section as a validated candidate, never inherits.
#[test]
fn only_a_child_binding_inherits_at_composition() {
    use crate::domain::inference_admission::{AdmissionConfig, GroupId, GroupPolicy};
    use crate::infrastructure::admission::{
        AuthorityDirectory, AuthorityServer, Negotiation, negotiate, write_admission_context,
    };
    use crate::infrastructure::provider_runtime_admission::AdmissionRuntimeProposal;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("authority");
    let group = GroupId::new("g").unwrap();
    let proposal = AdmissionRuntimeProposal {
        policy: AdmissionConfig {
            groups: std::collections::BTreeMap::from([(
                group.clone(),
                GroupPolicy {
                    capacity: 1,
                    reserve: 0,
                    min_interval_ms: 1,
                    queue_capacity: 1,
                    queue_timeout_ms: 1,
                    attempt_timeout_ms: 1,
                    fallback_base_ms: 1,
                    max_cooldown_ms: 1,
                },
            )]),
            aliases: std::collections::BTreeMap::from([("a".into(), group)]),
            max_scopes: 8,
            terminal_capacity: 8,
        },
        bindings: std::collections::BTreeMap::from([("openai".into(), "a".into())]),
    };
    let server = rt
        .block_on(AuthorityServer::start(
            AuthorityDirectory::open(&directory).unwrap(),
            proposal,
        ))
        .unwrap();
    let mut config = Config::default();
    config.admission = serde_json::from_str(&format!(
        r#"{{"directory":{:?},"groups":{{"g":{{"capacity":1,"reserve":0,"min_interval_ms":1,"queue_capacity":1,"queue_timeout_ms":1,"attempt_timeout_ms":1,"fallback_base_ms":1,"max_cooldown_ms":1}}}},"aliases":{{"a":"g"}},"bindings":{{"openai":"a"}}}}"#,
        directory.to_string_lossy()
    ))
    .unwrap();
    let root = negotiate(Negotiation::Root {
        directory: directory.clone(),
    })
    .unwrap();
    let (inherit, candidate) = admission_inheritance(&root, &config);
    assert!(!inherit, "a root never inherits");
    assert_eq!(candidate.unwrap().bindings["openai"], "a");

    let credential = rt.block_on(root.connection().register_child()).unwrap();
    let context = temp.path().join("child.json");
    write_admission_context(&context, &root.endpoint(), &credential).unwrap();
    let child = negotiate(Negotiation::Child { context }).unwrap();
    // Whatever its own config says (here: a differing section), a child
    // inherits and offers no candidate to validate.
    config.admission.as_mut().unwrap().bindings.clear();
    let (inherit, candidate) = admission_inheritance(&child, &config);
    assert!(inherit, "a child always inherits");
    assert!(candidate.is_none());
    rt.block_on(server.shutdown());
}
