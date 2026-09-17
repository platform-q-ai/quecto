use super::*;
use serde_json::json;

#[test]
fn secret_shaped_keys_are_recognised_case_insensitively() {
    for key in [
        "api_key",
        "apiKey",
        "API_KEY",
        "token",
        "secret",
        "password",
        "openai_key",
        "refresh_token",
        "Access_Token",
    ] {
        assert!(is_secret_key(key), "{key}");
    }
    for key in ["model", "api_base", "keys", "tokens_per_minute", "effort"] {
        assert!(!is_secret_key(key), "{key}");
    }
}

#[test]
fn scalar_leaves_under_secret_keys_are_redacted_and_counted() {
    let mut document = json!({
        "providers": {
            "openai": {"api_key": "sk-1", "api_base": "https://x"},
            "other": {"token": 42, "password": true, "secret": null}
        },
        "list": [{"apiKey": "a"}, {"model": "m"}],
        "agents": {"defaults": {"model": "gpt"}}
    });
    assert_eq!(redact_secret_leaves(&mut document, None), 4);
    assert_eq!(
        document,
        json!({
            "providers": {
                "openai": {"api_key": REDACTED, "api_base": "https://x"},
                "other": {"token": REDACTED, "password": REDACTED, "secret": null}
            },
            "list": [{"apiKey": REDACTED}, {"model": "m"}],
            "agents": {"defaults": {"model": "gpt"}}
        })
    );
}

#[test]
fn a_bare_leaf_is_redacted_by_the_key_it_was_read_under() {
    let mut leaf = json!("sk-1");
    assert_eq!(redact_secret_leaves(&mut leaf, Some("api_key")), 1);
    assert_eq!(leaf, json!(REDACTED));
    let mut model = json!("gpt");
    assert_eq!(redact_secret_leaves(&mut model, Some("model")), 0);
    assert_eq!(model, json!("gpt"));
    let mut unknown = json!("x");
    assert_eq!(redact_secret_leaves(&mut unknown, None), 0);
}

#[test]
fn an_object_under_a_secret_key_is_walked_not_replaced() {
    let mut document = json!({"secret": {"api_key": "k", "note": "n"}});
    assert_eq!(redact_secret_leaves(&mut document, None), 1);
    assert_eq!(
        document,
        json!({"secret": {"api_key": REDACTED, "note": "n"}})
    );
}
