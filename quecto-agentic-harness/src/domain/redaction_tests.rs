use super::*;

#[test]
fn scrubs_named_and_prefixed_secrets() {
    assert_eq!(
        redact_secrets("Authorization: Bearer abc.def"),
        "Authorization: [REDACTED]"
    );
    assert_eq!(redact_secrets("sk-livedeadbeef0001"), "[REDACTED]");
    assert_eq!(redact_secrets("token=hunter2"), "[REDACTED]");
    assert_eq!(redact_secrets("--api-key=topsecret"), "--[REDACTED]");
}

#[test]
fn scrubs_positional_provider_tokens() {
    assert_eq!(redact_secrets("AKIAIOSFODNN7EXAMPLE"), "[REDACTED]");
    assert_eq!(
        redact_secrets("ghp_0123456789abcdef0123456789abcdef0123"),
        "[REDACTED]"
    );
    assert_eq!(redact_secrets("xoxb-0123456789-abcdefghij"), "[REDACTED]");
}

#[test]
fn preserves_non_secret_text() {
    assert_eq!(redact_secrets("ls -la /tmp"), "ls -la /tmp");
    assert_eq!(redact_secrets("usage_limit_reached"), "usage_limit_reached");
}

#[test]
fn url_userinfo_is_redacted_wherever_a_url_appears() {
    assert_eq!(
        redact_url_userinfo("--repo https://user:ghp_secret@host/x/y is unreachable"),
        "--repo https://***@host/x/y is unreachable"
    );
    assert_eq!(
        redact_url_userinfo("ssh://git@example.test:2222/r.git and https://t0k3n@h/p"),
        "ssh://***@example.test:2222/r.git and https://***@h/p"
    );
    for untouched in [
        "https://host/x/y",
        "git@github.com:org/repo.git",
        "https://host/path?mail=a@b",
        "",
    ] {
        assert_eq!(redact_url_userinfo(untouched), untouched);
    }
}

#[test]
fn an_unencoded_at_in_the_password_is_redacted_to_the_path() {
    assert_eq!(
        redact_url_userinfo("https://u:p@ss@host/x/y"),
        "https://***@host/x/y"
    );
}

/// #2241: a provider key prefix only starts a key at the start of a word,
/// so ordinary words and names that contain one mid-word are left intact.
#[test]
fn a_key_prefix_inside_a_word_is_not_a_key() {
    for text in [
        "task-runner01",
        "task-board-lock",
        "myproj-task-runner01",
        "disk-usage",
        "risk-assessment",
        "Ask-questionsfreely",
        "flask-sqlalchemy-utils",
        "whisk-0123456789abcdef",
        "\\disk-usage",
        "%3Ddisk-usage",
    ] {
        assert_eq!(redact_secrets(text), text, "{text}");
    }
}

/// #2241: real key shapes are redacted wherever a word may start: at the
/// start of the text, after whitespace, punctuation, `_` or `-`; the text
/// around them is kept.
#[test]
fn real_key_shapes_are_redacted_at_every_word_start() {
    let keys = [
        "sk-0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKL",
        "sk-proj-Ab3dEf6hIj9kLm2nOp5qRs8tUv1wXy4z_-Ab3dEf6hIj9kLm2nOp5qRs8t",
        "sk-ant-api03-Ab3dEf6hIj9kLm2nOp5qRs8tUv1wXy4zAb3dEf6hIj9kLm2nOp5q-AAAA",
        "sk-ant-oat01-Ab3dEf6hIj9kLm2nOp5qRs8tUv1wXy4z",
        "sk-or-v1-0123456789abcdef0123456789abcdef0123456789abcdef",
        "sk-livedeadbeef0001",
        "SK-LIVEDEADBEEF0001",
        "AKIAIOSFODNN7EXAMPLE",
        "ghp_0123456789abcdef0123456789abcdef0123",
        "gho_0123456789abcdef0123456789abcdef0123",
        "xoxb-0123456789-abcdefghij",
        "AIzaSyA0123456789abcdefghijklmnopq",
    ];
    for key in keys {
        for (before, after) in [
            ("", ""),
            ("key ", " end"),
            ("\"", "\""),
            ("'", "'"),
            ("KEY=", ""),
            ("x:", ""),
            ("(", ")"),
            ("/", "/"),
            ("line\n", "\n"),
            ("MY_", ""),
            ("my-", ""),
            ("é", ""),
            // #2249 review: a key in JSON-escaped or URL-encoded text, or
            // glued to a digit or to a non-ASCII letter that case-folds
            // to an ASCII one, is still a key.
            ("line\\n", ""),
            ("a\\t", ""),
            ("a\\r", ""),
            ("a\\b", ""),
            ("a\\f", ""),
            ("a\\u000a", ""),
            ("a\\u00E9", ""),
            ("a\\u00ff", ""),
            ("\\\"", "\\\""),
            ("k%3D", ""),
            ("k%3a", ""),
            ("k%20", ""),
            ("0", ""),
            ("2", ""),
            ("\u{17F}", ""),
            ("\u{212A}", ""),
        ] {
            let text = format!("{before}{key}{after}");
            let redacted = redact_secrets(&text);
            assert!(!redacted.contains(key), "{text:?} -> {redacted:?}");
            assert!(redacted.contains("[REDACTED]"), "{text:?} -> {redacted:?}");
            assert!(redacted.ends_with(after), "{text:?} -> {redacted:?}");
        }
        assert_eq!(
            redact_secrets(&format!("key {key} end")),
            "key [REDACTED] end"
        );
    }
    assert_eq!(
        redact_secrets("a sk-aaaaaaaaaa,sk-bbbbbbbbbb b"),
        "a [REDACTED],[REDACTED] b"
    );
}

/// #2241: a named span keeps matching inside an identifier: an environment
/// name such as `GITHUB_TOKEN=` or `DB_PASSWORD:` is a secret label however
/// it is prefixed.
#[test]
fn a_named_secret_label_inside_an_identifier_is_still_redacted() {
    assert_eq!(redact_secrets("GITHUB_TOKEN=abc123"), "GITHUB_[REDACTED]");
    assert_eq!(redact_secrets("DB_PASSWORD: hunter2"), "DB_[REDACTED]");
    assert_eq!(
        redact_secrets("OPENAI_API_KEY=whatever"),
        "OPENAI_[REDACTED]"
    );
    assert_eq!(
        redact_secrets("Authorization: Bearer abc.def"),
        "Authorization: [REDACTED]"
    );
}

/// #2249 review: the texts a key really travels in — an escaped JSON
/// body, a shell `echo -e`, a URL-encoded query, a concatenated dump —
/// are redacted, keeping the escape or encoding in front of the key.
#[test]
fn a_key_in_escaped_or_encoded_text_is_redacted() {
    let key = "sk-proj-Ab3dEf6hIj9kLm2nOp5qRs8tUv1wXy4z";
    for (text, expected) in [
        (
            format!(r#"{{"message":"env dump:\n{key}"}}"#),
            r#"{"message":"env dump:\n[REDACTED]"}"#.to_owned(),
        ),
        (
            format!(r#""key\t{key}""#),
            r#""key\t[REDACTED]""#.to_owned(),
        ),
        (
            format!(r#"echo -e "a\n{key}""#),
            r#"echo -e "a\n[REDACTED]""#.to_owned(),
        ),
        (format!(r#""{key}""#), r#""[REDACTED]""#.to_owned()),
        (format!("k%3D{key}"), "k%3D[REDACTED]".to_owned()),
        (
            "x-goog-api-key%3AAIzaSyA0123456789abcdefghijklmnopq".to_owned(),
            "x-goog-api-key%3A[REDACTED]".to_owned(),
        ),
        ("0sk-livedeadbeef0001".to_owned(), "0[REDACTED]".to_owned()),
        (
            "2ghp_0123456789abcdef0123456789abcdef0123".to_owned(),
            "2[REDACTED]".to_owned(),
        ),
        (
            "\u{17F}sk-livedeadbeef0001".to_owned(),
            "\u{17F}[REDACTED]".to_owned(),
        ),
        (
            "Kghp_0123456789abcdef0123456789abcdef0123".to_owned(),
            "K[REDACTED]".to_owned(),
        ),
        (
            "\u{212A}ghp_0123456789abcdef0123456789abcdef0123".to_owned(),
            "\u{212A}[REDACTED]".to_owned(),
        ),
    ] {
        assert_eq!(redact_secrets(&text), expected, "{text:?}");
    }
}

/// #2249 review: only `sk-` is held to the word-start rule; the other
/// prefixed shapes match wherever they appear, as they always have.
#[test]
fn only_the_sk_prefix_needs_a_word_start() {
    for (text, expected) in [
        ("thighs_0123456789abcdefghij", "thi[REDACTED]"),
        ("boxoxb-0123456789-abcdefghij", "bo[REDACTED]"),
        ("plaiza0123456789abcdefghijkl", "pl[REDACTED]"),
        ("slovakia0123456789ABCD", "slov[REDACTED]"),
        ("unbearer abc.def", "un[REDACTED]"),
    ] {
        assert_eq!(redact_secrets(text), expected, "{text:?}");
    }
}

/// The lead written back in front of a redacted `sk-` key is one of the
/// allowed word starts, and nothing else.
#[test]
fn a_key_lead_is_one_of_the_allowed_word_starts() {
    for lead in [
        "", " ", "-", "_", "0", "\u{17F}", "\\n", "\\f", "\\u00aF", "%3D", "%20",
    ] {
        assert!(is_key_lead(lead), "{lead:?}");
    }
    for lead in [
        "a", "Z", "\\x", "\\u00g0", "%3", "%G0", "  ", "\\nn", "%3D%3D",
    ] {
        assert!(!is_key_lead(lead), "{lead:?}");
    }
}
