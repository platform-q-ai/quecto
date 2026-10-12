use super::*;

#[test]
fn an_issue_number_is_from_one_to_ten_digits() {
    assert_eq!(IssueNumber::new(1).map(IssueNumber::get), Ok(1));
    assert!(IssueNumber::new(MAX_ISSUE_NUMBER).is_ok());
    for number in [0, MAX_ISSUE_NUMBER + 1, u64::MAX] {
        assert_eq!(
            IssueNumber::new(number).unwrap_err().field,
            "issue",
            "{number}"
        );
    }
}

/// A reference is `#n`, `owner/repo#n` or an https URL: a host of
/// dot-joined labels, the last not all digits, a port from 1 to 65535,
/// then the characters RFC 3986 allows, one `#` at most and every `%` an
/// escape.
#[test]
fn a_reference_is_an_issue_or_an_https_url() {
    let label = "a".repeat(63);
    let fine = [
        "#1".to_string(),
        "#9999999999".into(),
        "o/r#1".into(),
        "https://a".into(),
        "https://a-b.c1".into(),
        "https://1.a".into(),
        format!("https://{label}.com"),
        "https://a:1".into(),
        "https://a:65535/x".into(),
        "https://a/%2Fb%e9".into(),
        "https://a?q=1&r=%20".into(),
        "https://a#frag".into(),
        "https://a#123".into(),
        "https://a/b#42".into(),
        "https://a/!$&'()*+,;=:@-._~?/x".into(),
        "https://a.0xg".into(),
        "https://a:000443".into(),
    ];
    for text in fine {
        assert!(Reference::parse(&text).is_ok(), "{text}");
    }
    let bad = [
        "#0".to_string(),
        "#01".into(),
        "#12345678901".into(),
        "2470".into(),
        "a/b/c#1".into(),
        "./repo#1".into(),
        "owner/..#1".into(),
        "own er/repo#1".into(),
        "http://example.com".into(),
        "https://".into(),
        "https://a..b".into(),
        "https://a.".into(),
        "https://a-".into(),
        "https://-a".into(),
        format!("https://{label}a.com"),
        "https://1.2.3.4".into(),
        "https://example.123".into(),
        "https://0x7f000001".into(),
        "https://1.0x1".into(),
        "https://a.0X".into(),
        "https://a:0000000".into(),
        "https://[::1]".into(),
        "https://u:p@a".into(),
        "https://a:1@b".into(),
        "https://user@a".into(),
        "https://a:0".into(),
        "https://a:99999".into(),
        "https://a:".into(),
        "https://a:8x".into(),
        "https://a/%zz".into(),
        "https://a/%2".into(),
        "https://a/%".into(),
        "https://a/b#c#d".into(),
        "https://a/b c".into(),
        "https://a/b|c".into(),
        "https://a/b`c".into(),
        "https://a/b\\c".into(),
        "https://a/b\"c".into(),
        "https://a/b<c".into(),
        "https://a/b{c".into(),
        "https://a/b^c".into(),
        "https://ex\u{e4}mple.com".into(),
        String::new(),
    ];
    for text in bad {
        assert_eq!(
            Reference::parse(&text).unwrap_err().field,
            "reference",
            "{text:?}"
        );
    }
    let url = format!(
        "https://example.com/{}",
        "a".repeat(MAX_REFERENCE_BYTES - 20)
    );
    assert_eq!(url.len(), MAX_REFERENCE_BYTES);
    assert!(Reference::parse(&url).is_ok(), "the longest reference");
    assert!(Reference::parse(&format!("{url}a")).is_err());
}

/// The key makes two the same pointer: a repository and a URL's host in
/// any case, port 443 or none, a port's leading zeros, an empty path or
/// `/`, an escape's hex in either case. A URL's path keeps its case.
#[test]
fn a_references_key_names_what_it_points_at() {
    let key = |text: &str| Reference::parse(text).unwrap().key().to_string();
    for (one, other) in [
        ("Owner/Repo#1", "owner/repo#1"),
        ("https://EXAMPLE.com/a", "https://example.com/a"),
        ("https://a:443/x", "https://a/x"),
        ("https://a:0443/x", "https://a/x"),
        ("https://a:08080/x", "https://a:8080/x"),
        ("https://a", "https://a/"),
        ("https://a?q", "https://a/?q"),
        ("https://a#f", "https://a/#f"),
        ("https://a/%2f", "https://a/%2F"),
        ("https://a/%7e", "https://a/~"),
        ("https://a/%41b", "https://a/Ab"),
        ("https://a/?", "https://a/"),
        ("https://a/#", "https://a/"),
        ("https://a/?#", "https://a/"),
        ("https://a/b/./c", "https://a/b/c"),
        ("https://a/b/../c", "https://a/c"),
        ("https://a/b/c/..", "https://a/b/"),
        ("https://a/../b", "https://a/b"),
        ("https://a/b/%2e%2E/c", "https://a/c"),
        ("https://a:000443/x", "https://a/x"),
    ] {
        assert_eq!(key(one), key(other), "{one} {other}");
    }
    for (one, other) in [
        ("https://a/A", "https://a/a"),
        ("#1", "owner/repo#1"),
        ("owner/repo#1", "owner/repo#2"),
        ("https://a:8080/x", "https://a/x"),
        ("https://a/%2F", "https://a//"),
        ("https://a/?q", "https://a/"),
        ("https://a/b/c", "https://a/b/c/"),
    ] {
        assert_ne!(key(one), key(other), "{one} {other}");
    }
    let shown = Reference::parse("Owner/Repo#7").unwrap();
    assert_eq!(shown.as_str(), "Owner/Repo#7", "shown as given");
    assert_eq!(
        shown.points(),
        Points::Elsewhere(IssueNumber::new(7).unwrap())
    );
    assert_eq!(
        Reference::parse("#7").unwrap().points(),
        Points::Own(IssueNumber::new(7).unwrap())
    );
    assert_eq!(
        Reference::parse("https://a").unwrap().points(),
        Points::Page
    );
}

/// A refusal echoes at most the first 64 characters of what it refused.
#[test]
fn a_refusal_echoes_a_capped_reference() {
    let long = format!("not a reference {}", "x".repeat(200));
    let problem = Reference::parse(&long).unwrap_err().problem;
    let shown: String = long.chars().take(64).collect();
    assert!(problem.contains(&format!("{shown}…")), "{problem}");
    assert!(!problem.contains(&long), "{problem}");
    let short = Reference::parse("nope").unwrap_err().problem;
    assert!(short.starts_with("\"nope\""), "{short}");
}
