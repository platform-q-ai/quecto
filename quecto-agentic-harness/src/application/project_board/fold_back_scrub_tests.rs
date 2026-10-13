//! S4 review rounds 2 and 3: what the scrubber keeps of an address in an
//! owner or evidence part, and what a link holds.
use super::tests::kept_line;
use super::*;

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
fn an_address_is_never_retargeted_past_its_authority() {
    assert_eq!(
        kept_line("https://good.example?to=a@b.co", "abc"),
        Some("[link](https://good.example) at abc".into())
    );
    // A fragment that is no anchor is dropped.
    assert_eq!(
        kept_line("https://good.example#@evil.example/x", "abc"),
        Some("[link](https://good.example) at abc".into())
    );
    // Userinfo only inside the authority, and only as the scrub writes it.
    assert_eq!(
        address("https://***@good.example/x"),
        Some("https://good.example/x".into())
    );
    assert_eq!(address("https://evil@good.example/x"), None);
    assert_eq!(address("https://***@evil@good.example/x"), None);
    assert_eq!(
        address("https://good.example?to=a@b.co"),
        Some("https://good.example".into())
    );
    assert_eq!(
        address("https://good.example#@evil.example/x"),
        Some("https://good.example".into())
    );
    assert_eq!(
        address("https://good.example#L10-L20"),
        Some("https://good.example#L10-L20".into())
    );
    assert_eq!(
        address("https://h.example/app;jsessionid=A1"),
        Some("https://h.example/app".into())
    );
    assert_eq!(address("https://***"), None);
    assert_eq!(address("https://h.example:80x/a"), None);
}

#[test]
fn a_port_and_an_at_sign_in_the_path_are_kept_and_trailing_marks_left_outside() {
    assert_eq!(
        address("https://x.com:8443/a"),
        Some("https://x.com:8443/a".into())
    );
    assert_eq!(
        address("https://x.com/u@v"),
        Some("https://x.com/u@v".into())
    );
    assert_eq!(
        address("https://x.com/a:b"),
        Some("https://x.com/a:b".into())
    );
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

/// What [`scrubbed`] makes of each row's text, row by row.
fn scrubs(rows: &[(&str, &str)]) {
    for (raw, kept) in rows {
        assert_eq!(scrubbed(raw), *kept, "{raw}");
    }
}

#[test]
fn an_authority_is_kept_only_as_a_host_and_port_else_the_whole_address_goes() {
    scrubs(&[
        ("postgres://app:S3cr#t@db:5432/app", "postgres://***"),
        ("redis://:p?ss@cache", "redis://***"),
        ("postgres://u:ab/cd@db", "postgres://***"),
        ("https://u:ab/cd@h", "https://***"),
        // A password of digits reads as a port: an `@` after it gives it away.
        ("https://u:1234/cd@h", "https://***"),
        ("postgres://app:1234#t@db/app", "postgres://***"),
        ("`postgres://app:S3cr#t@db:5432/app`", "`postgres://***`"),
        ("postgres://u:p@h/db", "postgres://***@h/db"),
        // The last `@` ends the userinfo.
        ("https://u:p@ss@host/x", "https://***@host/x"),
        ("https://good.example?to=a@b.co", "https://good.example"),
        ("https://x.com:8443/a", "https://x.com:8443/a"),
    ]);
    assert_eq!(kept_line("https://u:ab/cd@h", "abc"), None);
    assert_eq!(
        kept_line("https://good.example?to=a@b.co", "abc"),
        Some("[link](https://good.example) at abc".into())
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
        ("Postgres://u:p@h/db", "Postgres://***@h/db"),
        (
            "bucket.s3.amazonaws.com/x?sig=y",
            "bucket.s3.amazonaws.com/x",
        ),
        (
            "`bucket.s3.amazonaws.com/x?sig=y`",
            "`bucket.s3.amazonaws.com/x`",
        ),
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
    // A parenthesis opened in what was dropped is closed with it.
    scrubs(&[
        ("[t](https://h?a=(b))", "[t](https://h)"),
        ("(see https://h/p?a=(b))", "(see https://h/p)"),
        ("[t](https://h/a(b)c)", "[t](https://h/a(b)c)"),
    ]);
    assert_eq!(
        kept_line("[t](https://h.example?a=(b))", "abc"),
        Some("[t](https://h.example) at abc".into())
    );
}
