//! Contract for the `RefreshInputsLoader` port (#1846): every load is a
//! fresh read of the base directory, a catalogue file that cannot be
//! enumerated is an error naming why (never a partial set), and a valid file
//! yields one refreshable per OpenAI-compatible api-key provider.
use std::sync::Arc;

use quecto::application::catalogue::ports::RefreshInputsLoader;
use quecto::infrastructure::catalogue_refresh_inputs::FileRefreshInputs;

fn under_test(dir: &std::path::Path) -> Arc<dyn RefreshInputsLoader> {
    Arc::new(FileRefreshInputs::new(dir))
}

#[test]
fn each_load_reads_the_current_file_and_a_malformed_file_is_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let loader = under_test(tmp.path());
    assert!(loader.load().unwrap().refreshables().is_empty());

    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"openrouter":{"api":"openai-completions","baseUrl":"https://or.test/v1","apiKey":"sk-or","models":[]}}}"#,
    )
    .unwrap();
    let loaded = loader.load().unwrap();
    let ids: Vec<&str> = loaded.refreshables().iter().map(|s| s.id()).collect();
    assert_eq!(ids, vec!["openrouter"]);

    std::fs::write(tmp.path().join("models.json"), "{ not json").unwrap();
    let Err(error) = loader.load() else {
        panic!("a malformed file cannot be enumerated");
    };
    assert!(!error.is_empty());
}
