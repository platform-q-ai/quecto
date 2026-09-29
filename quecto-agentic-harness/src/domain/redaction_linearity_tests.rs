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

/// The shortest of `runs` redactions of `text`: the least disturbed by a
/// loaded runner.
fn fastest_redaction(text: &str, runs: usize) -> Duration {
    (0..runs)
        .map(|_| {
            let started = Instant::now();
            std::hint::black_box(redact_secrets(std::hint::black_box(text)));
            started.elapsed()
        })
        .min()
        .expect("at least one run")
}

/// The smallest time a ratio is taken against, so a sub-millisecond
/// small run cannot turn scheduler noise into a failure.
const TIME_FLOOR: Duration = Duration::from_millis(20);

/// The most a mebibyte of any unit may take. Every unit takes under 1.5 s
/// in a debug build on an idle machine; the bound leaves room for a
/// loaded CI runner, and a quadratic rule (minutes) is far past it.
const LARGE_BOUND: Duration = Duration::from_secs(5);

/// A unit's two timings, and how the large one grew over the small one.
struct Growth {
    small: Duration,
    large: Duration,
    ratio: f64,
}

impl Growth {
    /// Time `small` and `large`, the best of `runs` each.
    fn measure(small: &str, large: &str, runs: usize) -> Self {
        let (small, large) = (
            fastest_redaction(small, runs),
            fastest_redaction(large, runs),
        );
        let ratio = large.as_secs_f64() / small.max(TIME_FLOOR).as_secs_f64();
        Self {
            small,
            large,
            ratio,
        }
    }

    /// The fastest of two measurements at each size.
    fn measure_min(first: Self, again: Self) -> Self {
        let (small, large) = (first.small.min(again.small), first.large.min(again.large));
        let ratio = large.as_secs_f64() / small.max(TIME_FLOOR).as_secs_f64();
        Self {
            small,
            large,
            ratio,
        }
    }

    /// Whether it is linear: well under six times as long for four times
    /// the text, and inside [`LARGE_BOUND`].
    fn is_linear(&self) -> bool {
        self.ratio < 6.0 && self.large < LARGE_BOUND
    }
}

/// Linear time: a text four times as long takes well under six times as
/// long (a quadratic rule takes sixteen), and a mebibyte of any unit
/// redacts inside [`LARGE_BOUND`] even in a debug build. Each unit is
/// timed once, and a unit that looks slow is timed again, twice more at
/// each size, keeping the fastest of the three (one far past the bound
/// is no scheduling hiccup, and fails at once).
#[test]
fn every_rule_family_redacts_adversarial_text_in_linear_time() {
    const SMALL: usize = 256 * 1024;
    const LARGE: usize = 1024 * 1024;
    let mut failures = Vec::new();
    for unit in ADVERSARIAL_UNITS {
        let small = unit.repeat(SMALL / unit.len());
        let large = unit.repeat(LARGE / unit.len());
        let first = Growth::measure(&small, &large, 1);
        let growth = match (first.is_linear(), first.large < LARGE_BOUND * 3) {
            (true, _) | (false, false) => first,
            (false, true) => {
                let again = Growth::measure(&small, &large, 2);
                Growth::measure_min(first, again)
            }
        };
        eprintln!(
            "{unit:?}: 256 KiB {:?}, 1 MiB {:?}, ratio {:.2}",
            growth.small, growth.large, growth.ratio
        );
        if growth.ratio >= 12.0 {
            failures.push(format!("{unit:?} is quadratic: ratio {:.2}", growth.ratio));
        } else if growth.ratio >= 6.0 {
            failures.push(format!(
                "{unit:?} grows too fast: ratio {:.2}",
                growth.ratio
            ));
        }
        if growth.large >= LARGE_BOUND {
            failures.push(format!("{unit:?}: 1 MiB took {:?}", growth.large));
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
