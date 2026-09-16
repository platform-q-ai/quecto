//! Contract for the `RuntimeSnapshotSource` port (#1847): nothing before the
//! first successful composition; afterwards the generation the runtime
//! composer published, catalogue and provider together, re-read on every
//! call so a recomposition is honoured.
use std::sync::Arc;

use quecto::application::catalogue::ports::RuntimeSnapshotSource;
use quecto::infrastructure::catalogue_registry::{runtime_store_for, snapshot_store_for};

fn under_test(dir: &std::path::Path) -> Arc<dyn RuntimeSnapshotSource> {
    Arc::new(runtime_store_for(dir))
}

fn compose(dir: &std::path::Path) {
    std::fs::write(
        dir.join("models.json"),
        r#"{"providers":{"wired":{"api":"openai-completions","apiKey":"sk-wired","baseUrl":"https://wired.test/v1","models":[{"id":"m"}]}}}"#,
    )
    .unwrap();
    quecto::interface::catalogue_runtime::compose_and_publish_runtime(
        &quecto::infrastructure::config::Config::default(),
        dir,
        &reqwest::Client::new(),
    )
    .expect("composition succeeds");
}

#[test]
fn nothing_before_composition_then_the_published_generation() {
    let tmp = tempfile::tempdir().unwrap();
    let source = under_test(tmp.path());
    assert!(source.current_runtime().is_none());
    compose(tmp.path());
    let first = source.current_runtime().expect("published");
    assert_eq!(
        first.generation(),
        snapshot_store_for(tmp.path()).current().generation(),
        "catalogue and runtime are one generation"
    );
    assert!(
        first
            .catalogue
            .entries()
            .iter()
            .any(|e| e.reference().qualified_id() == "wired/m")
    );
    compose(tmp.path());
    let second = source.current_runtime().expect("republished");
    assert!(
        second.generation() > first.generation(),
        "a recomposition is honoured"
    );
}
