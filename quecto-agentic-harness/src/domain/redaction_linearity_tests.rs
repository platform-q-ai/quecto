//! #2304 review round 5: every rule family redacts in time linear in the
//! text, however adversarial (H1, M1), and no text of multibyte
//! characters, quotes and backslashes panics or redacts to anything but
//! itself a second time.

use super::*;
use std::time::{Duration, Instant};

/// One adversarial unit per rule family and shape, each repeated to fill
/// a text: labels (bare, quoted, keyed, XML, a path's), prefixed keys, a
/// PEM block, URLs, and every command-line shape.
const ADVERSARIAL_UNITS: &[&str] = &[
    // Named labels (H1: a label with no enclosing quote, far into a line).
    "token=\"a ",
    "token=\"a password='b ",
    " token=a",
    "password: ",
    "\"password\": \" ",
    "password:\" ",
    "'token=",
    "{\\\"token\\\":\\\"",
    "<password>",
    "/passwd: ",
    "token = Some(\"",
    "token=a\"",
    "token=a\"b",
    "PWD=/",
    "Pwd=a;",
    "password: Bearer ",
    "token = x.y(",
    // Prefixed keys and blocks.
    "Bearer ",
    "Bearer%20",
    "--oauth2-bearer a ",
    "sk-",
    "eyJa.b",
    "github_pat_",
    "-----BEGIN PRIVATE KEY-----",
    // URLs.
    "x://a:b",
    "x://a:b@",
    "https://a@",
    "https://h:1/@",
    "x://a",
    // Command-line shapes (M1: a login with no user, far into a line).
    "x login -p a ",
    "x login -u -p ",
    "login -u -p ",
    "; x login -p a ",
    "x login -p a -u ",
    "curl -u a ",
    "curl a ",
    "http -a ",
    "--token ",
    "--creds=",
    "--password ",
    "-passin ",
    "mysql -p",
    "sshpass -p ",
    "redis-cli -a ",
    "gh secret set -b ",
    "Authorization: basic ",
    "Cookie: \"",
    "A_PASS=",
    "A_KEY=",
    "SECRET_KEY=",
    "SSHPASS=",
    "aws_secret_access_key ",
    "twine -p ",
    "zip -P ",
    "7z -p",
    "sqlcmd -P ",
    "ssh-keygen -N ",
    "doctl -t ",
    "vault login ",
    "cargo login ",
    "set -x A_TOKEN ",
];

/// The shortest of two redactions of `text`: the least disturbed by a
/// loaded runner.
fn fastest_redaction(text: &str) -> Duration {
    (0..2)
        .map(|_| {
            let started = Instant::now();
            std::hint::black_box(redact_secrets(std::hint::black_box(text)));
            started.elapsed()
        })
        .min()
        .expect("two runs")
}

/// The smallest time a ratio is taken against, so a sub-millisecond
/// small run cannot turn scheduler noise into a failure.
const TIME_FLOOR: Duration = Duration::from_millis(20);

/// Linear time: a text four times as long takes well under six times as
/// long (a quadratic rule takes sixteen), and a mebibyte of any unit
/// redacts in under the absolute bound even in a debug build.
#[test]
fn every_rule_family_redacts_adversarial_text_in_linear_time() {
    const SMALL: usize = 256 * 1024;
    const LARGE: usize = 1024 * 1024;
    let bound = Duration::from_secs(2);
    let mut failures = Vec::new();
    for unit in ADVERSARIAL_UNITS {
        let small = unit.repeat(SMALL / unit.len());
        let large = unit.repeat(LARGE / unit.len());
        let (small_time, large_time) = (fastest_redaction(&small), fastest_redaction(&large));
        let ratio = large_time.as_secs_f64() / small_time.max(TIME_FLOOR).as_secs_f64();
        eprintln!("{unit:?}: 256 KiB {small_time:?}, 1 MiB {large_time:?}, ratio {ratio:.2}");
        if ratio >= 12.0 {
            failures.push(format!("{unit:?} is quadratic: ratio {ratio:.2}"));
        } else if ratio >= 6.0 {
            failures.push(format!("{unit:?} grows too fast: ratio {ratio:.2}"));
        }
        if large_time >= bound {
            failures.push(format!("{unit:?}: 1 MiB took {large_time:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A small deterministic generator (xorshift64*), so the fuzz corpus is
/// the same on every run and machine.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next() % n as u64).expect("below n fits")
    }
}

/// The fragments fuzz strings are built from: labels, flags, quotes,
/// backslashes, wrappers, URL parts and multibyte characters.
const FRAGMENTS: &[&str] = &[
    "token",
    "password",
    "pwd",
    "api_key",
    "secret",
    "=",
    ":",
    " ",
    "\t",
    "\"",
    "'",
    "\\",
    "\\\"",
    "é",
    "日本",
    "🙂",
    "\u{17F}",
    "Some(",
    ")",
    "(",
    "[",
    "]",
    "b",
    "$",
    "@",
    "://",
    "https",
    "x",
    "-p",
    "-u",
    "login",
    "\n",
    "\r",
    "Bearer ",
    "sk-",
    "abc",
    "Ab1",
    "123",
    "%20",
    "<password>",
    "</",
    ">",
    "-----BEGIN PRIVATE KEY-----",
    "-----END PRIVATE KEY-----",
    "curl ",
    "a:b",
    ";",
    "&",
    "#",
    ",",
    "{",
    "}",
    "--token ",
    "eyJ",
    ".",
    "_KEY=",
    "set -x ",
    "_TOKEN ",
    "mysql ",
    "?",
    "/",
    "::",
    "<",
    "Secret",
    "[REDACTED]",
    "Cookie: ",
    "-",
];

/// About two thousand strings of fragments: none panics, and each
/// redacts to itself a second time.
#[test]
fn fuzzed_text_never_panics_and_redacts_to_itself() {
    let mut rng = Rng(0x2304_5EED_0000_0005);
    let mut failures = Vec::new();
    for _ in 0..2000 {
        let count = 1 + rng.below(24);
        let text: String = (0..count)
            .map(|_| FRAGMENTS[rng.below(FRAGMENTS.len())])
            .collect();
        let once = redact_secrets(&text);
        let twice = redact_secrets(&once);
        if twice != once {
            failures.push(format!("{text:?}\n   once  {once:?}\n   twice {twice:?}"));
        }
    }
    assert!(
        failures.is_empty(),
        "{} not idempotent:\n{}",
        failures.len(),
        failures
            .iter()
            .take(20)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
