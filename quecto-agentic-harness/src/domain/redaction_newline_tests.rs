//! #2304 review round 4 (L4): no rule reaches across a newline. A label,
//! flag or assignment at a line's end with its would-be value on the next
//! line is left as written, for every rule family; a quoted value that
//! never closes on its line is redacted to the line's end only.

use super::*;

#[test]
fn no_rule_family_takes_a_value_from_the_next_line() {
    for text in [
        // Named labels, bare and JSON.
        "password:\n  file: /run/secrets/db",
        "token=\nnext",
        "\"password\":\n\"x\"",
        "  secret:\n    name: db",
        // Bearer.
        "Bearer\nabc.def",
        // A spaced secret flag, and the password flags.
        "tool --token\nabc",
        "tool --api-key\nabcdef0123456789",
        "tool --password\nabc",
        "openssl enc -pass\npass:x",
        // Authorization and cookie headers.
        "Authorization: Basic\nYWxpY2U6czNjcmV0",
        "Authorization:\nBasic YWxpY2U6czNjcmV0",
        "Authorization: token\nabc",
        "Cookie:\nsid=1",
        // A connection URL.
        "https://u:\npw@host",
        // The tool-scoped flags.
        "curl -u\nalice:pw",
        "mysql -p\npw",
        "sshpass -p\npw",
        "x login -p\npw",
        "redis-cli -a\npw",
        "gh secret set X --body\nabc",
        // Assignments.
        "DB_PASS=\npw",
        "SIGNING_KEY=\n0123456789abcdef0",
        "aws_secret_access_key\nwJalrXUtnFEMI/K7MDENG",
        "aws_secret_access_key:\nwJalrXUtnFEMI/K7MDENG",
    ] {
        assert_eq!(redact_secrets(text), text, "{text:?}");
    }
}

#[test]
fn a_quoted_value_open_at_a_lines_end_is_redacted_to_that_end_only() {
    for (text, expected) in [
        ("--password \"a b\nc\"", "--password \"[REDACTED]\nc\""),
        ("token: 'a b\nc'", "token: '[REDACTED]\nc'"),
        (
            "export DB_PASS=\"a\nb\"",
            "export DB_PASS=\"[REDACTED]\nb\"",
        ),
        ("'password: a b\nc'", "'password: [REDACTED]\nc'"),
        ("password: a b\nuser: c", "password: [REDACTED]\nuser: c"),
    ] {
        assert_eq!(redact_secrets(text), expected, "{text:?}");
    }
}

/// The gaps and false positives the round-four rules keep on purpose,
/// each pinned so a change to it is a decision.
#[test]
fn the_round_four_trade_offs_are_pinned() {
    for (text, expected) in [
        // A bare value stops at `@` (so `…token:ghs_…@host` keeps its
        // host): an unquoted password holding an `@` leaks its tail.
        ("password=p@ss", "password=[REDACTED]@ss"),
        // A secret flag takes the next word whatever it is.
        (
            "build --secret id=npm,src=/r/.npmrc .",
            "build --secret [REDACTED] .",
        ),
        // A label starting a line takes the rest of it, prose included.
        ("Token: expired, log in again", "Token: [REDACTED]"),
        // Mid-line, a bare value is one word.
        ("the password: two words", "the password: [REDACTED] words"),
    ] {
        assert_eq!(redact_secrets(text), expected, "{text:?}");
    }
}
