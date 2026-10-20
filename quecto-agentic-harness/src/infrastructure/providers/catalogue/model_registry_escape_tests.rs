//! #2617 review: what `models.json` names in a diagnostic (a provider key,
//! a declared api) is printed escaped, so it cannot carry a newline or an
//! ESC sequence onto the terminal.

use super::*;

fn skipped_errors(json: &str) -> Vec<String> {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("models.json");
    std::fs::write(&path, json).unwrap();
    ModelRegistry::load_registry_config(&path)
        .unwrap()
        .skipped
        .into_iter()
        .map(|block| block.error)
        .collect()
}

#[test]
fn a_skipped_providers_key_is_escaped() {
    let errors = skipped_errors(
        r#"{"providers":{
            "a\u001b[31m":{"api":"openai-completions","auth":{"mode":"vault"},"models":[{"id":"m"}]},
            "b\nforged":{"api":"openai-completions","streamIdleSeconds":0,"models":[{"id":"m"}]}
        }}"#,
    );
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert_eq!(
        errors[0],
        r"provider 'a\u{1b}[31m' declares an unknown auth mode (supported: apiKey, oauth); its models were skipped"
    );
    assert!(
        errors[1].starts_with(r"provider 'b\nforged' declares "),
        "{}",
        errors[1]
    );
    for error in &errors {
        assert!(!error.contains(['\u{1b}', '\n']), "{error:?}");
    }
}

#[test]
fn an_unknown_api_is_quoted_escaped() {
    assert_eq!(
        ModelRegistryError::UnknownApi("a\u{1b}[0m\nb".to_string()).to_string(),
        r"unknown api 'a\u{1b}[0m\nb' in models registry"
    );
}
