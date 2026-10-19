//! Contract for the auth capability's `ExternalCredentialSource` (#1840)
//! over an external `auth.json`: the OAuth entries for Anthropic then
//! OpenAI, with their expiry in seconds and OpenAI's account; anything
//! else is left out; an absent, unreadable or malformed file is refused,
//! naming the path where it helps. Reading writes nothing.
use base64::Engine;
use tempfile::TempDir;

use quecto::application::auth::ports::{ExternalCredentialSource, ExternalCredentialsError};
use quecto::domain::auth::provider::LoginProvider;
use quecto::infrastructure::auth::auth_json::AuthJsonFile;

fn file_holding(content: &str) -> (TempDir, AuthJsonFile) {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("auth.json");
    std::fs::write(&path, content).unwrap();
    (dir, AuthJsonFile::at(path))
}

fn jwt_naming(account: &str) -> String {
    let claims = serde_json::json!({
        "https://api.openai.com/auth": { "chatgpt_account_id": account }
    });
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(serde_json::to_vec(&claims).unwrap());
    format!("header.{payload}.signature")
}

#[test]
fn the_oauth_entries_come_in_import_order_with_seconds_and_accounts() {
    let jwt = jwt_naming("acct-fake");
    let (dir, source) = file_holding(
        &serde_json::json!({
            "openai": { "type": "oauth", "access": jwt, "refresh": "fake-ro", "expires": 7_000_999 },
            "anthropic": { "type": "oauth", "access": "fake-a", "refresh": "fake-ra", "expires": 5_000 },
        })
        .to_string(),
    );
    let before = std::fs::read_dir(dir.path()).unwrap().count();
    let found = source.load().unwrap();
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].provider, LoginProvider::Anthropic);
    assert_eq!(found[0].access_token, "fake-a");
    assert_eq!(found[0].refresh_token, "fake-ra");
    assert_eq!(found[0].expires_at, 5);
    assert_eq!(found[0].account_id, None);
    assert_eq!(found[1].provider, LoginProvider::OpenAi);
    assert_eq!(found[1].expires_at, 7_000);
    assert_eq!(found[1].account_id.as_deref(), Some("acct-fake"));
    assert!(
        !format!("{found:?}").contains("fake-ra"),
        "secrets stay out of Debug"
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), before);
}

#[test]
fn only_oauth_entries_of_importable_providers_are_found() {
    let (_dir, source) = file_holding(
        r#"{"anthropic": {"type": "api", "key": "fake"},
            "openai": {"access": "fake-untyped"},
            "xai": {"type": "oauth", "access": "fake-x"},
            "groq": {"type": "oauth", "access": "fake-g"}}"#,
    );
    assert!(source.load().unwrap().is_empty());
}

#[test]
fn missing_fields_read_as_empty_and_unstated_expiry_as_zero() {
    let (_dir, source) = file_holding(r#"{"openai": {"type": "oauth"}}"#);
    let found = source.load().unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].access_token, "");
    assert_eq!(found[0].refresh_token, "");
    assert_eq!(found[0].expires_at, 0);
    assert_eq!(found[0].account_id, None);
}

#[test]
fn an_absent_file_is_not_found_at_its_path() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("auth.json");
    assert_eq!(
        AuthJsonFile::at(&path).load(),
        Err(ExternalCredentialsError::NotFound {
            path: path.display().to_string()
        })
    );
}

#[test]
fn an_unreadable_file_is_refused_naming_its_path() {
    let dir = TempDir::new().unwrap();
    // A directory exists but cannot be read as a file.
    let path = dir.path().join("auth.json");
    std::fs::create_dir(&path).unwrap();
    match AuthJsonFile::at(&path).load() {
        Err(ExternalCredentialsError::Unreadable { path: named, .. }) => {
            assert_eq!(named, path.display().to_string());
        }
        other => panic!("expected an unreadable file, got {other:?}"),
    }
}

#[test]
fn a_malformed_file_is_refused() {
    let (_dir, source) = file_holding("{ not json");
    assert!(matches!(
        source.load(),
        Err(ExternalCredentialsError::Malformed(_))
    ));
}
