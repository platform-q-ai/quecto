use std::path::Path;

use super::*;

fn layout() -> FlatSessionLayout {
    FlatSessionLayout::new("/base")
}

#[test]
fn cli_alpha_projects_to_the_exact_unchanged_flat_paths() {
    let alpha = SessionIdentity::named_cli("alpha").unwrap();
    let layout = layout();
    assert_eq!(layout.sessions_dir(), Path::new("/base/sessions"));
    assert_eq!(
        layout.session_file(&alpha),
        Path::new("/base/sessions/cli_alpha.json")
    );
    assert_eq!(
        layout.ownership_stamp(&alpha),
        Path::new("/base/sessions/cli_alpha.owner")
    );
    assert_eq!(
        layout.spill_file(&alpha),
        Path::new("/base/sessions/cli_alpha/spill.jsonl")
    );
}

#[test]
fn unsafe_and_unicode_keys_use_the_hex_sanitizer_like_before() {
    let layout = layout();
    let traversal = SessionIdentity::from_persisted_key("../etc");
    let expected = super::super::filename::sanitize_session_key("../etc");
    assert!(expected.starts_with("key_"));
    assert_eq!(
        layout.session_file(&traversal),
        Path::new("/base/sessions").join(format!("{expected}.json"))
    );
    assert_eq!(
        layout.spill_file(&traversal),
        Path::new("/base/sessions")
            .join(&expected)
            .join("spill.jsonl")
    );
    let unicode = SessionIdentity::from_persisted_key("chät");
    assert_eq!(
        layout.ownership_stamp(&unicode),
        Path::new("/base/sessions").join(format!(
            "{}.owner",
            super::super::filename::sanitize_session_key("chät")
        ))
    );
}

#[test]
fn ephemeral_identity_keeps_the_sanitized_empty_key_spill_file() {
    let layout = layout();
    let empty = super::super::filename::sanitize_session_key("");
    assert_eq!(empty, "key_");
    assert_eq!(
        layout.spill_file(&SessionIdentity::ephemeral()),
        Path::new("/base/sessions/key_/spill.jsonl")
    );
}

#[test]
fn distinct_identities_never_share_a_projection() {
    let layout = layout();
    let a = SessionIdentity::named_cli("a").unwrap();
    let b = SessionIdentity::named_cli("b").unwrap();
    let chat = SessionIdentity::fresh_chat(1, 2);
    let all = [&a, &b, &chat, &SessionIdentity::ephemeral()];
    for (i, left) in all.iter().enumerate() {
        for right in all.iter().skip(i + 1) {
            assert_ne!(layout.session_file(left), layout.session_file(right));
            assert_ne!(layout.ownership_stamp(left), layout.ownership_stamp(right));
            assert_ne!(layout.spill_file(left), layout.spill_file(right));
        }
    }
}

#[test]
fn record_allowlist_and_prefix_projection_match_the_list_filters() {
    assert!(FlatSessionLayout::is_session_record(Path::new(
        "/base/sessions/cli_alpha.json"
    )));
    assert!(!FlatSessionLayout::is_session_record(Path::new(
        "/base/sessions/cli_alpha.owner"
    )));
    assert!(!FlatSessionLayout::is_session_record(Path::new(
        "/base/sessions/cli_alpha"
    )));
    let layout = layout();
    assert_eq!(
        layout.record_name_prefix(&SessionKeyPrefix::new("chat-").unwrap()),
        "chat-"
    );
    assert_eq!(
        layout.record_name_prefix(&SessionKeyPrefix::new("cli:").unwrap()),
        "cli_"
    );
}

#[test]
fn layout_is_a_plain_comparable_value() {
    assert_eq!(FlatSessionLayout::new("/x"), FlatSessionLayout::new("/x"));
    assert_ne!(FlatSessionLayout::new("/x"), FlatSessionLayout::new("/y"));
    assert!(format!("{:?}", FlatSessionLayout::new("/x")).contains("/x/sessions"));
}
