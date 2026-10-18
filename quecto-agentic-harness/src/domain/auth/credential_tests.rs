use super::*;

fn credential(expires_at: Option<i64>) -> Credential {
    Credential {
        provider: "openai".to_string(),
        token: "fake-access-token".to_string(),
        method: AuthMethod::OAuth,
        expires_at,
        refresh_token: Some("fake-refresh-token".to_string()),
        account_id: Some("acct-fake".to_string()),
    }
}

#[test]
fn a_credential_without_an_expiry_never_expires() {
    let never = credential(None);
    assert!(!never.is_expired_at(i64::MIN));
    assert!(!never.is_expired_at(i64::MAX));
    assert_eq!(never.state_at(i64::MAX), CredentialState::Active);
}

#[test]
fn a_credential_expires_at_its_instant_not_a_second_before() {
    let dated = credential(Some(1_000));
    assert!(!dated.is_expired_at(999));
    assert!(dated.is_expired_at(1_000));
    assert!(dated.is_expired_at(1_001));
    assert_eq!(dated.state_at(999), CredentialState::Active);
    assert_eq!(dated.state_at(1_000), CredentialState::Expired);
}

#[test]
fn credential_states_read_as_the_status_words() {
    assert_eq!(CredentialState::Active.as_str(), "active");
    assert_eq!(CredentialState::Expired.as_str(), "expired");
}

#[test]
fn auth_methods_read_as_their_stored_names() {
    assert_eq!(AuthMethod::Token.as_str(), "token");
    assert_eq!(AuthMethod::OAuth.as_str(), "oauth");
}

#[test]
fn an_oauth_expiry_keeps_a_five_minute_margin() {
    assert_eq!(OAUTH_EXPIRY_MARGIN_SECS, 300);
    assert_eq!(oauth_expires_at(10_000, 3_600), 10_000 + 3_600 - 300);
    // A token issued already inside the margin is due for refresh at once.
    assert_eq!(oauth_expires_at(10_000, 0), 9_700);
}

#[test]
fn an_oauth_expiry_saturates_instead_of_overflowing() {
    assert_eq!(oauth_expires_at(i64::MAX, u64::MAX), i64::MAX - 300);
}

#[test]
fn debug_never_shows_the_token_or_the_refresh_token() {
    let shown = format!("{:?}", credential(Some(5)));
    assert!(!shown.contains("fake-access-token"), "{shown}");
    assert!(!shown.contains("fake-refresh-token"), "{shown}");
    assert!(shown.contains("[REDACTED]"), "{shown}");
    assert!(
        shown.contains("openai") && shown.contains("acct-fake"),
        "{shown}"
    );
}

#[test]
fn debug_shows_an_absent_refresh_token_as_none() {
    let mut plain = credential(None);
    plain.refresh_token = None;
    assert!(format!("{plain:?}").contains("refresh_token: None"));
}
