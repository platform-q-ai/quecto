use super::*;
use crate::domain::session::USER_CHAT_PREFIX;

#[test]
fn ephemeral_identity_is_the_empty_key_with_no_persisted_key() {
    let identity = SessionIdentity::ephemeral();
    assert!(identity.is_ephemeral());
    assert_eq!(identity.persisted_key(), None);
    assert_eq!(identity.runtime_key(), "");
}

#[test]
fn every_historical_persisted_key_category_round_trips_unchanged() {
    for key in [
        "cli:default",
        "cli:alpha",
        "chat-1700000000-1a2b",
        "telegram:12345",
        "weird key/with:slashes and spaces",
        "…unicode…",
        ".hidden",
        "..",
    ] {
        let identity = SessionIdentity::from_persisted_key(key);
        assert!(!identity.is_ephemeral(), "{key:?} is a persisted key");
        assert_eq!(identity.persisted_key(), Some(key));
        assert_eq!(identity.runtime_key(), key);
        assert_eq!(
            SessionIdentity::from_persisted_key(key.to_string()),
            identity,
            "owned and borrowed construction agree"
        );
    }
}

#[test]
fn from_persisted_key_keeps_the_exact_bytes_without_normalisation() {
    // No trimming, folding or validation: a padded key is a distinct key.
    let padded = SessionIdentity::from_persisted_key(" padded ");
    assert_eq!(padded.persisted_key(), Some(" padded "));
    assert_eq!(padded.runtime_key(), " padded ");
    assert_ne!(padded, SessionIdentity::from_persisted_key("padded"));
    assert_ne!(padded, SessionIdentity::from_persisted_key("PADDED"));
    let whitespace_only = SessionIdentity::from_persisted_key("  ");
    assert!(!whitespace_only.is_ephemeral());
    assert_eq!(whitespace_only.persisted_key(), Some("  "));
}

#[test]
fn the_empty_persisted_key_is_the_ephemeral_identity() {
    // A stored file with no snapshot header reads back as the empty key
    // today; the typed identity classifies that as ephemeral, nothing else.
    assert_eq!(
        SessionIdentity::from_persisted_key(""),
        SessionIdentity::ephemeral()
    );
}

#[test]
fn named_cli_applies_the_existing_session_name_allowlist() {
    assert_eq!(
        SessionIdentity::named_cli("alpha").unwrap().runtime_key(),
        "cli:alpha"
    );
    assert_eq!(
        SessionIdentity::named_cli("a-b_C9")
            .unwrap()
            .persisted_key(),
        Some("cli:a-b_C9")
    );
    // `-` is an admitted name (the interface treats it as the ephemeral
    // marker before ever building an identity from it).
    assert!(SessionIdentity::named_cli("-").is_ok());
    for rejected in ["", "with space", "slash/", "colon:x", "dot.", "ünïcode"] {
        let err = SessionIdentity::named_cli(rejected).unwrap_err();
        assert_eq!(
            err.to_string(),
            "session error: session name must contain only alphanumeric, '-', or '_'",
            "{rejected:?}"
        );
        assert!(!SessionIdentity::is_valid_cli_name(rejected));
    }
}

#[test]
fn fresh_chat_uses_the_domain_chat_key_shape() {
    let identity = SessionIdentity::fresh_chat(1_700_000_000, 0xabc);
    assert_eq!(identity.runtime_key(), "chat-1700000000-abc");
    assert!(identity.runtime_key().starts_with(USER_CHAT_PREFIX));
    assert_eq!(identity, SessionIdentity::fresh_chat(1_700_000_000, 0xabc));
}

#[test]
fn key_prefix_admits_identities_by_raw_key_prefix_only() {
    let chats = SessionKeyPrefix::new(USER_CHAT_PREFIX).unwrap();
    assert_eq!(chats.as_str(), "chat-");
    assert!(chats.admits(&SessionIdentity::fresh_chat(1, 2)));
    assert!(!chats.admits(&SessionIdentity::named_cli("x").unwrap()));
    assert!(!chats.admits(&SessionIdentity::ephemeral()));
    assert_eq!(
        SessionKeyPrefix::new("").unwrap_err().to_string(),
        "session error: session key prefix must not be empty"
    );
}

#[test]
fn user_chat_admits_only_a_chat_prefixed_key_of_allowlisted_characters() {
    let identity = SessionIdentity::user_chat("chat-1750000000-abc").unwrap();
    assert_eq!(identity.runtime_key(), "chat-1750000000-abc");
    assert_eq!(identity.persisted_key(), Some("chat-1750000000-abc"));
    // The bare prefix is a (degenerate) chat key of allowlisted characters.
    assert!(SessionIdentity::user_chat("chat-").is_ok());
    for rejected in ["cli:x", "chat-with space", "chat-a/b", "", "chatx"] {
        let err = SessionIdentity::user_chat(rejected).unwrap_err();
        assert!(
            err.to_string()
                .contains("session name must contain only alphanumeric, '-', or '_'"),
            "{rejected:?}: {err}"
        );
    }
}

#[test]
fn spill_id_is_a_plain_typed_wrapper() {
    let id = SpillId::new("turn20:bash:0");
    assert_eq!(id.as_str(), "turn20:bash:0");
    assert_eq!(id, SpillId::new(String::from("turn20:bash:0")));
}
