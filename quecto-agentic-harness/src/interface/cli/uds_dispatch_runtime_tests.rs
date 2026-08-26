//! UDS-surface tests for the structured model-selection outcome (#1573):
//! `selection_status` is the payload `set_model` responses carry, so these
//! pin the wire-visible shape of every selection verdict.

use super::selection_status;

fn write_models_json(dir: &std::path::Path, body: &str) {
    std::fs::write(dir.join("models.json"), body).unwrap();
}

fn compose(dir: &std::path::Path) {
    crate::interface::catalogue_runtime::compose_and_publish_runtime(
        &crate::infrastructure::config::Config::default(),
        dir,
        &reqwest::Client::new(),
    )
    .expect("composition succeeds");
}

#[test]
fn no_composed_runtime_keeps_the_legacy_response_shape() {
    let tmp = tempfile::tempdir().unwrap();
    assert_eq!(selection_status(tmp.path(), "openai-api/gpt-5"), None);
}

#[test]
fn set_model_payload_carries_the_structured_selection_verdicts() {
    let tmp = tempfile::tempdir().unwrap();
    write_models_json(
        tmp.path(),
        r#"{"providers":{
            "udsish":{"api":"openai-completions","apiKey":"sk-uds","baseUrl":"https://api.example.test/v1",
                "models":[{"id":"uds-model","name":"UDS Model"}]},
            "udskeyless":{"api":"openai-completions",
                "models":[{"id":"bare-model","name":"Bare Model"}]}
        }}"#,
    );
    compose(tmp.path());

    let ok = selection_status(tmp.path(), "udsish/uds-model").expect("runtime published");
    assert_eq!(ok["status"], "ok");
    assert_eq!(ok["provider"], "udsish");
    assert!(ok["generation"].as_u64().unwrap() >= 1);

    let unknown = selection_status(tmp.path(), "udsish/no-such-model").expect("runtime published");
    assert_eq!(unknown["status"], "unknown_model");
    assert_eq!(unknown["model"], "udsish/no-such-model");

    let unrunnable =
        selection_status(tmp.path(), "udskeyless/bare-model").expect("runtime published");
    assert_eq!(unrunnable["status"], "not_runnable");
    assert_eq!(unrunnable["model"], "udskeyless/bare-model");
    let reasons: Vec<&str> = unrunnable["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap())
        .collect();
    assert!(reasons.contains(&"missing-credential"), "got {reasons:?}");
}
