use super::*;
use crate::domain::auth::credential::AuthMethod;

fn stated(expires_at: Option<i64>, refresh_token: Option<&str>) -> Credential {
    Credential {
        provider: "openai".to_string(),
        token: "fake-access".to_string(),
        method: AuthMethod::OAuth,
        expires_at,
        refresh_token: refresh_token.map(str::to_string),
        account_id: None,
    }
}

#[test]
fn only_an_entry_with_an_access_token_is_importable() {
    assert!(importable("fake-access"));
    assert!(!importable(""));
}

#[test]
fn an_empty_refresh_token_is_none() {
    assert_eq!(refresh_token(String::new()), None);
    assert_eq!(
        refresh_token("fake-refresh".to_string()),
        Some("fake-refresh".to_string())
    );
}

#[test]
fn a_stated_credential_already_expired_as_stored_is_refreshed_first() {
    // Stored at its expiry less the margin: due once that has passed.
    assert!(import_refresh_due(
        &stated(Some(1_000), Some("fake-refresh")),
        1_000
    ));
    assert!(import_refresh_due(
        &stated(Some(1_000), Some("fake-refresh")),
        5_000
    ));
    assert!(!import_refresh_due(
        &stated(Some(1_000), Some("fake-refresh")),
        999
    ));
}

#[test]
fn without_a_refresh_token_nothing_is_refreshed() {
    assert!(!import_refresh_due(&stated(Some(0), None), 5_000));
}

#[test]
fn a_token_inside_its_margin_is_refreshed_rather_than_stored_expired() {
    // 100 s left on the server: stored at -200 s, so already expired.
    let now = 10_000;
    let server_expiry = now + 100;
    let as_stored = stated(
        Some(crate::domain::auth::credential::oauth_expires_at_absolute(
            server_expiry,
        )),
        Some("fake-refresh"),
    );
    assert!(as_stored.is_expired_at(now));
    assert!(import_refresh_due(&as_stored, now));
}
