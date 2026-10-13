//! S4 review rounds 2 to 4: what the scrubber keeps of an address in an
//! owner or evidence part, and what a link holds. The board is public:
//! when a rule is in doubt, it scrubs.
use super::tests::kept_line;
use super::*;

/// What [`scrubbed`] makes of each row's text, row by row.
fn scrubs(rows: &[(&str, &str)]) {
    for (raw, kept) in rows {
        assert_eq!(scrubbed(raw), *kept, "{raw}");
    }
}

#[test]
fn an_address_keeps_no_query_wherever_it_stands() {
    let rows = [
        (
            "`https://acct.blob.core.windows.net/c/b?sv=2024&sig=AbC%2Bsecret`",
            "`https://acct.blob.core.windows.net/c/b`",
        ),
        ("`http://h.example/p?sig=s3cr3t`", "`http://h.example/p`"),
        (
            "[x](https://h.example/p?sig=s3cr3t)",
            "[x](https://h.example/p)",
        ),
        (
            "https://h.example/p?sig=s3cr3t#frag",
            // A fragment after a dropped query goes with it.
            "[link](https://h.example/p)",
        ),
        ("https://x.com/a?", "[link](https://x.com/a)?"),
    ];
    for (artifact, line) in rows {
        let kept = kept_line(artifact, "abc");
        assert_eq!(kept, Some(format!("{line} at abc")), "{artifact}");
        assert!(kept.is_some_and(|kept| !kept.contains("sig=") && !kept.contains("sv=")));
    }
}

#[test]
fn any_at_sign_in_an_address_gives_the_whole_address_away() {
    scrubs(&[
        // Cases review found: an `@` in a password, a path or a query.
        ("postgres://app:S3@cr/t@db/app", "postgres://***"),
        ("postgres://app:S3@cr#t@db:5432/app", "postgres://***"),
        ("https://abc/def@github.com/o/r.git", "https://***"),
        ("https://abc?def@github.com/x", "https://***"),
        ("postgres://app:S3cr#t@db:5432/app", "postgres://***"),
        ("redis://:p?ss@cache", "redis://***"),
        ("postgres://u:ab/cd@db", "postgres://***"),
        ("https://u:1234/cd@h", "https://***"),
        ("postgres://u:p@h/db", "postgres://***"),
        ("Postgres://u:p@h/db", "Postgres://***"),
        ("https://u:p@ss@host/x", "https://***"),
        ("`postgres://app:S3cr#t@db:5432/app`", "`postgres://***`"),
        // A nested address: the outer one holds its `@`.
        (
            "https://web.archive.org/web/2020/https://u:p@h/x",
            "https://***",
        ),
        // The accepted cost: npm scopes, vite paths and `@` in a query.
        ("https://www.npmjs.com/package/@scope/pkg", "https://***"),
        ("http://localhost:5173/@vite/client", "http://***"),
        ("https://good.example?to=a@b.co", "https://***"),
        ("https://good.example#@evil.example/x", "https://***"),
    ]);
    // Such a line holds a bare address, which the codec refuses.
    assert_eq!(kept_line("https://u:ab/cd@h", "abc"), None);
}

#[test]
fn an_authority_is_kept_only_when_it_is_unambiguous() {
    scrubs(&[
        ("https://x.com:8443/a", "https://x.com:8443/a"),
        // A port is digits, at least one.
        ("https://h:/x", "https://***"),
        ("https://h.example:80x/a", "https://***"),
        // IPv6 and non-ASCII hosts are over-scrubbed.
        ("https://[::1]/x", "https://***"),
        ("https://bücher.example/x", "https://***"),
    ]);
}

#[test]
fn a_word_holding_an_at_sign_before_a_host_is_scrubbed_whole() {
    scrubs(&[
        ("oauth2:glpat-abc@gitlab.com/o/r.git", "***"),
        ("`oauth2:glpat-abc@gitlab.com/o/r.git`", "`***`"),
        ("mailed ada@example.com.", "mailed ***."),
        ("user@bucket.s3.amazonaws.com/x?sig=y", "***"),
        // An `@` before no host is left.
        ("worker@1 done", "worker@1 done"),
    ]);
}

#[test]
fn a_unicode_space_inside_an_address_ends_it_and_leaves_no_password() {
    scrubs(&[
        ("https://u:pa\u{a0}ss@h/x", "https://*** ***"),
        ("https://u:pa\u{2003}ss@h.example/x", "https://*** ***"),
    ]);
    let line = kept_line("https://u:pa\u{a0}ss@h/x", "abc");
    assert!(
        line.as_deref().is_none_or(|line| !line.contains("ss")),
        "{line:?}"
    );
}

#[test]
fn only_an_anchor_fragment_straight_after_the_path_is_kept() {
    scrubs(&[
        ("https://h.example/p#code=abc", "https://h.example/p"),
        ("https://h.example/p#sig=x", "https://h.example/p"),
        ("https://h.example/p?token=ab#cd", "https://h.example/p"),
        ("https://h.example/p#L10-L20", "https://h.example/p#L10-L20"),
        (
            "https://h.example/p#readme.md",
            "https://h.example/p#readme.md",
        ),
        // An empty fragment is no anchor.
        ("https://h/p#", "https://h/p"),
    ]);
}

#[test]
fn matrix_parameters_and_an_encoded_query_mark_end_the_path() {
    scrubs(&[
        (
            "https://h.example/app;jsessionid=ABC123",
            "https://h.example/app",
        ),
        (
            "https://h.example/app;jsessionid=A?q=1#f",
            "https://h.example/app",
        ),
        ("https://h.example/p%3Fsig=x", "https://h.example/p"),
        ("https://h.example/p%3fsig=x", "https://h.example/p"),
    ]);
}

#[test]
fn a_scheme_in_any_case_or_none_still_loses_its_query() {
    scrubs(&[
        ("HTTPS://h?SIG=x", "HTTPS://h"),
        (
            "bucket.s3.amazonaws.com/x?sig=y",
            "bucket.s3.amazonaws.com/x",
        ),
        (
            "`bucket.s3.amazonaws.com/x?sig=y`",
            "`bucket.s3.amazonaws.com/x`",
        ),
        // A bare host with no path loses its query too.
        ("api.example.com?sig=y", "api.example.com"),
        ("README.md#install", "README.md#install"),
        ("//h.example/p?sig=y", "//h.example/p"),
        ("see `//h?sig=y` here", "see `//h` here"),
        ("src/api.rs", "src/api.rs"),
        ("a // comment", "a // comment"),
    ]);
}

#[test]
fn the_marks_that_close_a_dropped_query_are_given_back() {
    for mark in [".", ",", ";", ":", "!", "]", "\"", "'", "`", ")", ">", "?"] {
        assert_eq!(
            scrubbed(&format!("see https://h/p?sig=S{mark}")),
            format!("see https://h/p{mark}"),
            "{mark}"
        );
    }
    // A parenthesis opened in what was dropped is closed with it; one
    // opened and closed there leaves the text's own close.
    scrubs(&[
        ("[t](https://h?a=(b))", "[t](https://h)"),
        ("(see https://h/p?a=(b))", "(see https://h/p)"),
        ("(see https://h/p?a=(b)c)", "(see https://h/p)"),
        ("[t](https://h/a(b)c)", "[t](https://h/a(b)c)"),
    ]);
    assert_eq!(
        kept_line("[t](https://h.example?a=(b))", "abc"),
        Some("[t](https://h.example) at abc".into())
    );
}

#[test]
fn a_link_holds_an_unambiguous_address_only() {
    assert_eq!(
        address("https://good.example#L10-L20"),
        Some("https://good.example#L10-L20".into())
    );
    assert_eq!(
        address("https://h.example/app;jsessionid=A1"),
        Some("https://h.example/app".into())
    );
    assert_eq!(
        address("https://good.example?to=x"),
        Some("https://good.example".into())
    );
    assert_eq!(
        address("https://x.com:8443/a"),
        Some("https://x.com:8443/a".into())
    );
    assert_eq!(
        address("https://x.com/a:b"),
        Some("https://x.com/a:b".into())
    );
    for refused in [
        "https://***",
        "https://evil@good.example/x",
        "https://x.com/u@v",
        "https://h.example:80x/a",
        "https://h:/x",
    ] {
        assert_eq!(address(refused), None, "{refused}");
    }
    assert_eq!(
        kept_line("https://x.com:8443/a", "abc"),
        Some("[link](https://x.com:8443/a) at abc".into())
    );
    for mark in [";", ":", "!", "?", ".", ","] {
        assert_eq!(
            kept_line(&format!("https://x.com/a{mark}"), "abc"),
            Some(format!("[link](https://x.com/a){mark} at abc")),
            "{mark}"
        );
    }
}

#[test]
fn a_close_that_balances_an_open_in_the_address_stays_in_its_link() {
    assert_eq!(
        kept_line(
            "https://en.wikipedia.org/wiki/Rust_(programming_language)",
            "abc"
        ),
        Some("[link](https://en.wikipedia.org/wiki/Rust_%28programming_language%29) at abc".into())
    );
    assert_eq!(
        kept_line(
            "(https://en.wikipedia.org/wiki/Rust_(programming_language))",
            "abc"
        ),
        Some(
            "([link](https://en.wikipedia.org/wiki/Rust_%28programming_language%29)) at abc".into()
        )
    );
    assert_eq!(
        kept_line("(https://x.com/a)", "abc"),
        Some("([link](https://x.com/a)) at abc".into())
    );
}
