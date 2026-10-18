//! One credential-presence authority (#1842, #2451): whether a stored
//! credential has expired is decided only by `domain::auth::credential`.
//! The status use case and the provider slots the catalogue's
//! `CredentialStatusPort` reads both ask that rule; no other code compares
//! a credential's expiry itself.
use super::teardown_authority::{production_code, production_files};

/// The one owner of the expiry comparison.
const OWNER: &str = "src/domain/auth/credential.rs";

/// The two readers of credential presence, and the domain rule each asks.
const READERS: &[(&str, &str)] = &[
    (
        "src/application/auth/use_cases/inspect_auth_status.rs",
        ".state_at(",
    ),
    (
        "src/infrastructure/auth/provider_slots.rs",
        ".is_expired_at(",
    ),
];

/// A comparison against a credential's `expires_at`, either way round
/// (`now >= c.expires_at`, `expires_at < now`); `->` and `=>` are no
/// comparisons.
fn compares_expiry(line: &str) -> bool {
    static PATTERN: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    PATTERN
        .get_or_init(|| {
            regex::Regex::new(r"(?:^|[^-=<>])[<>]=?\s*[\w.]*expires_at\b|\bexpires_at\s*[<>]")
                .expect("expiry comparison pattern")
        })
        .is_match(line)
}

#[test]
fn only_the_domain_compares_a_credentials_expiry() {
    let offenders: Vec<String> = production_files()
        .into_iter()
        .filter(|file| file != OWNER)
        .flat_map(|file| {
            production_code(&file)
                .into_iter()
                .filter(|(_, line)| compares_expiry(line))
                .map(move |(number, line)| format!("{file}:{number}: {}", line.trim()))
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "compare a credential's expiry through domain::auth::credential: {offenders:#?}"
    );
    assert!(
        production_code(OWNER)
            .iter()
            .any(|(_, line)| line.contains("now >= expires_at")),
        "the owner keeps the rule"
    );
}

#[test]
fn every_presence_reader_asks_the_domain_rule() {
    for (reader, rule) in READERS {
        assert!(
            production_code(reader)
                .iter()
                .any(|(_, line)| line.contains(rule)),
            "{reader} must decide presence through domain::auth ({rule})"
        );
    }
}

#[test]
fn the_expiry_scan_sees_both_spellings() {
    assert!(compares_expiry("if now >= c.expires_at {"));
    assert!(compares_expiry("expires_at < now"));
    assert!(compares_expiry("x.expires_at > now"));
    assert!(!compares_expiry("expires_at: Some(at),"));
    assert!(!compares_expiry(".field(\"expires_at\", &self.expires_at)"));
    assert!(!compares_expiry(
        "fn f() -> Option<i64> { self.expires_at }"
    ));
    assert!(!compares_expiry("Some(at) => expires_at,"));
}
