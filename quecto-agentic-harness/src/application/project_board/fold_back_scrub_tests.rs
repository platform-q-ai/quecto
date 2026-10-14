//! S4 review rounds 2 to 7: what the scrubber keeps of an address in an
//! owner or evidence part, and what a link holds. The board is public:
//! when a rule is in doubt, it scrubs.
use super::tests::kept_line;
use super::*;

/// What [`scrubbed`] makes of each row's text, row by row.
fn scrubs(rows: &[(&str, &str)]) {
    for (raw, kept) in rows {
        assert_eq!(
            scrubbed(raw, MAX_LINE_CHARS).as_deref(),
            Some(*kept),
            "{raw}"
        );
    }
}

/// Each of `raws` is in doubt: its line is dropped.
fn doubted(raws: &[&str]) {
    for raw in raws {
        assert_eq!(scrubbed(raw, MAX_LINE_CHARS), None, "{raw}");
    }
}

#[test]
fn an_address_keeps_no_query_wherever_it_stands() {
    let rows = [
        (
            "`https://acct.blob.core.windows.net/c/b?sv=2024&sig=AbC%2Bs3cr3t`",
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
fn an_at_sign_outside_an_address_drops_the_line_but_for_an_exempt_word() {
    doubted(&[
        "root:secret@tcp(127.0.0.1:3306)/db",
        "user:pass@db:5432/app",
        "user:pass@localhost:6379",
        "u:p@[::1]:5432/x",
        "u:p@h\u{f4}st.com/x",
        "pkg@1.2.3",
        "@types/node",
        "git@github.com:o/r",
        "mailed ada@example.com.",
        "oauth2:glpat-abc@gitlab.com/o/r.git",
        "`oauth2:glpat-abc@gitlab.com/o/r.git`",
        "user@bucket.s3.amazonaws.com/x?sig=y",
        // Fullwidth and small at signs are at signs.
        "ada\u{ff20}example.com",
        "ada\u{fe6b}example.com",
        "worker@1x",
        // A numeric password before a space: its tail holds the `@`.
        "https://user:1234 5678@host.com/",
    ]);
    scrubs(&[
        ("worker@1 done", "worker@1 done"),
        ("by worker-2@12.", "by worker-2@12."),
        ("HEAD@{1}", "HEAD@{1}"),
        ("`main@{0}`", "`main@{0}`"),
    ]);
}

#[test]
fn userinfo_outside_a_named_scheme_is_never_kept() {
    // No scheme the address pattern reads: the at sign drops the line.
    doubted(&["x_://u:p@h", "1://tok@db", "://tok@db"]);
}

#[test]
fn a_line_naming_a_credential_word_is_dropped_in_any_case_or_markup() {
    doubted(&[
        "Authorization: Basic dXNlcjpwYXNzd29yZA==",
        "Authorization: token-xyz",
        "authorization",
        "basic dXNlcjpw",
        "BASIC YWRhOjEyMzQ1Njc4",
        "Bearer abcdef0123456789",
        "bearer abc",
        "`Bearer` `eyJhbGciOi`",
        "**Bearer** eyJhbGciOi",
        "Bearer: eyJhbGciOi",
        "Bearer=eyJhbGciOi",
        "Bear\u{200b}er eyJhbGciOi",
        "bas\u{feff}ic dXNl",
        // `basic` before anything but a letter, or at the end.
        "Basic\u{200b}dXNlcjpw",
        "Basic:dXNlcjpw",
        "Basic\u{a0}dXNlcjpw",
        "curl -H Basic",
        // Compatibility forms fold to the word (NFKC).
        "\u{ff50}\u{ff41}\u{ff53}\u{ff53}\u{ff57}\u{ff4f}\u{ff52}\u{ff44}=x",
        "password: hunter2",
        "Passwd x",
        "pwd=x",
        "client_secret x",
        "an access token",
        "api_key x",
        "APIKEY x",
        "api-key x",
        "credentials file",
        "private_key.pem",
        "private-key.pem",
        // The accepted cost: a line that merely mentions one.
        "basic CI-check passed",
        "token budget",
    ]);
    // No keyword: kept.
    scrubs(&[
        ("a basically fine run", "a basically fine run"),
        ("src/api.rs", "src/api.rs"),
    ]);
}

#[test]
fn a_part_past_its_limit_is_dropped_never_cut() {
    for raw in [
        "user:s3cretPassw0rd@tcp(db)/app",
        "https://deploy:12345678@h.com",
        "dXNlcjpwYXNzd29yZA",
    ] {
        // Cut short of its `@`, it would read clean: so it is never cut.
        assert_eq!(scrubbed(raw, raw.chars().count() - 1), None, "{raw}");
    }
    let at = "a".repeat(MAX_LINE_CHARS);
    assert_eq!(scrubbed(&at, MAX_LINE_CHARS).as_deref(), Some(at.as_str()));
    assert_eq!(scrubbed(&format!("{at}a"), MAX_LINE_CHARS), None);
    // In characters, not bytes.
    let wide = "\u{10400}".repeat(MAX_LINE_CHARS);
    assert_eq!(
        scrubbed(&wide, MAX_LINE_CHARS).as_deref(),
        Some(wide.as_str())
    );
}

#[test]
fn a_percent_encoded_at_sign_is_an_at_sign() {
    doubted(&[
        "https%3A%2F%2Fadmin%3As3cretPW%40db.example.com%2Fx",
        "oauth2:TOKEN%40gitlab.com/x",
        "oauth2:glpat%40gitlab.com/x",
        "u:p%40host.com",
        "u:p%2540host.com",
        "u:p%25252540host.com",
        "u:p%40",
        "worker%401",
    ]);
    scrubs(&[
        // An address holding one is written whole.
        ("https://u:p%40h.example/x", "https://***"),
        ("https://h.example/a%2540b", "https://***"),
        // Another escape is no at sign.
        ("https://h.example/a%20b", "https://h.example/a%20b"),
    ]);
}

#[test]
fn an_exempt_word_is_judged_after_redaction() {
    // A key glued to `@1` is no `worker@1`: once redacted, it is dropped.
    doubted(&["sk-abcdefgh12@1", "AKIA0123456789ABCDEF@2"]);
}

#[test]
fn a_unicode_space_inside_an_address_ends_it_and_drops_the_line() {
    doubted(&[
        "https://u:pa\u{a0}ss@h/x",
        "https://u:pa\u{2003}ss@h.example/x",
    ]);
    assert_eq!(kept_line("https://u:pa\u{a0}ss@h/x", "abc"), None);
}

#[test]
fn each_line_break_keeps_its_line_two_lines() {
    for break_ in ['\n', '\r', '\u{2028}', '\u{2029}', '\u{85}', '\u{b}'] {
        assert_eq!(
            kept_line(&format!("src{break_}api.rs"), "abc"),
            None,
            "{break_:?}"
        );
    }
}

#[test]
fn only_a_short_anchor_fragment_is_kept_and_any_other_drops_the_line() {
    scrubs(&[
        ("https://h.example/p#L10-L20", "https://h.example/p#L10-L20"),
        (
            "https://h.example/p#readme.md",
            "https://h.example/p#readme.md",
        ),
        // A short anchor after a dropped query goes with the query.
        ("https://h.example/p?t=ab#cd", "https://h.example/p"),
    ]);
    let long = format!("https://h.example/p#{}", "a".repeat(MAX_FRAGMENT_CHARS + 1));
    doubted(&[
        "https://h.example/p#code=abc",
        "https://h.example/p#sig=x",
        "https://h.example/p?a=b#sig=x",
        // An empty fragment is no anchor.
        "https://h/p#",
        &long,
    ]);
    let longest = format!("https://h.example/p#{}", "a".repeat(MAX_FRAGMENT_CHARS));
    scrubs(&[(&longest, &longest)]);
}

#[test]
fn a_doubly_encoded_path_or_a_bare_host_with_a_semicolon_drops_the_line() {
    doubted(&[
        "https://h.example/a%253Fsig=x",
        "https://h.example/a%25",
        "h.example/app;jsessionid=A1",
    ]);
    scrubs(&[("https://h.example/a%20b", "https://h.example/a%20b")]);
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
            scrubbed(&format!("see https://h/p?sig=S{mark}"), MAX_LINE_CHARS),
            Some(format!("see https://h/p{mark}")),
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

#[test]
fn a_long_run_of_parentheses_is_judged_in_one_pass() {
    let started = std::time::Instant::now();
    let (opens, closes) = ("(".repeat(32_768), ")".repeat(32_768));
    let linked = linked_word(&format!("https://h.example/a{opens}{closes}"));
    assert!(
        linked.starts_with("[link](https://h.example/a%28"),
        "{}",
        &linked[..40]
    );
    assert_eq!(balanced(&format!("h?{opens}"), "h", &closes), "");
    let kept = scrubbed(&format!("https://h.example?a={opens}{closes}"), usize::MAX);
    assert_eq!(kept.as_deref(), Some("https://h.example"));
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn a_part_holding_an_invisible_format_character_is_dropped() {
    for invisible in [
        '\u{ad}',    // soft hyphen
        '\u{200e}',  // left-to-right mark
        '\u{2061}',  // function application
        '\u{34f}',   // combining grapheme joiner
        '\u{fe0f}',  // variation selector 16
        '\u{e0041}', // tag latin capital A
        '\u{200b}',  // zero-width space
        '\u{feff}',  // zero-width no-break space
    ] {
        assert_eq!(
            scrubbed(&format!("src/api{invisible}.rs"), MAX_LINE_CHARS),
            None,
            "{:?}",
            invisible
        );
    }
    // Visible marks and letters are kept.
    scrubs(&[("caf\u{e9} \u{4e2d}\u{6587}", "caf\u{e9} \u{4e2d}\u{6587}")]);
}
