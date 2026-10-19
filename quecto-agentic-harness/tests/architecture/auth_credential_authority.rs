//! One expiry rule (#1842, #2451): a stored credential's `expires_at` is
//! read only where the allowlist says — the domain rule that decides
//! expiry, and the store's mapping to and from `credentials.json`. The
//! status use case and the provider slots behind the catalogue's
//! `CredentialStatusPort` both ask the domain rule.
//!
//! Exact and parsed, not a pattern: any field read `….expires_at` and any
//! pattern binding an `expires_at` field counts, whatever is done with it
//! (`c.expires_at.is_some_and(…)`, `matches!(c.expires_at, …)`,
//! `Credential { expires_at, .. }`). Building a credential (a struct
//! literal's `expires_at: …`) is no read.
use std::collections::BTreeSet;

use syn::visit::Visit;

use super::teardown_authority::production_files;

/// The files that may read a credential's `expires_at`.
const MAY_READ_EXPIRY: &[&str] = &[
    // The rule: `is_expired_at`, `state_at`.
    "src/domain/auth/credential.rs",
    // The boundary mapping of the domain credential to its file record.
    "src/infrastructure/auth/credential_store.rs",
    // The external credential's own redacting `Debug` (#1840).
    "src/application/auth/ports/external_credentials.rs",
    // The import rule's input: the external credential's stated expiry,
    // turned into the stored one by `oauth_expires_at_absolute` (#1840).
    "src/application/auth/use_cases/import_external_credentials.rs",
];

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

#[derive(Default)]
struct ExpiryReads(usize);

/// Whether `ident` names the field, raw (`r#expires_at`) or not.
fn names_expiry(ident: &proc_macro2::Ident) -> bool {
    use syn::ext::IdentExt;
    ident.unraw() == "expires_at"
}

/// How many identifiers in `tokens` name the field, at any depth. String
/// literals are literals, not identifiers, so they never count.
fn expiry_idents(tokens: proc_macro2::TokenStream) -> usize {
    tokens
        .into_iter()
        .map(|token| match token {
            proc_macro2::TokenTree::Ident(ident) => usize::from(names_expiry(&ident)),
            proc_macro2::TokenTree::Group(group) => expiry_idents(group.stream()),
            proc_macro2::TokenTree::Punct(_) | proc_macro2::TokenTree::Literal(_) => 0,
        })
        .sum()
}

impl<'ast> Visit<'ast> for ExpiryReads {
    fn visit_expr_field(&mut self, field: &'ast syn::ExprField) {
        if matches!(&field.member, syn::Member::Named(name) if names_expiry(name)) {
            self.0 += 1;
        }
        syn::visit::visit_expr_field(self, field);
    }

    fn visit_field_pat(&mut self, field: &'ast syn::FieldPat) {
        if matches!(&field.member, syn::Member::Named(name) if names_expiry(name)) {
            self.0 += 1;
        }
        syn::visit::visit_field_pat(self, field);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        // A macro's tokens are opaque until expanded (`field!(c,
        // expires_at)`, `matches!(c.expires_at, …)`): any identifier in
        // them naming the field counts.
        self.0 += expiry_idents(mac.tokens.clone());
        syn::visit::visit_macro(self, mac);
    }
}

/// How many times `source` reads an `expires_at` field.
fn expiry_reads(source: &str) -> usize {
    let file = syn::parse_file(source).expect("a crate source parses");
    let mut reads = ExpiryReads::default();
    reads.visit_file(&file);
    reads.0
}

#[test]
fn only_the_allowlist_reads_a_credentials_expiry() {
    let readers: BTreeSet<String> = production_files()
        .into_iter()
        .filter(|file| {
            let source = std::fs::read_to_string(file).expect("read a production file");
            expiry_reads(&source) > 0
        })
        .collect();
    let allowed: BTreeSet<String> = MAY_READ_EXPIRY.iter().map(|s| s.to_string()).collect();
    let stray: Vec<_> = readers.difference(&allowed).collect();
    assert!(
        stray.is_empty(),
        "read a credential's expiry through domain::auth::credential: {stray:#?}"
    );
    let stale: Vec<_> = allowed.difference(&readers).collect();
    assert!(
        stale.is_empty(),
        "allowlisted but reads no expiry: {stale:#?}"
    );
}

#[test]
fn every_presence_reader_asks_the_domain_rule() {
    for (reader, rule) in READERS {
        let source = std::fs::read_to_string(reader).expect("read the reader");
        assert!(
            source.contains(rule),
            "{reader} must decide presence through domain::auth ({rule})"
        );
    }
}

#[test]
fn every_spelling_of_an_expiry_read_counts() {
    for read in [
        "fn f(c: &C) -> bool { c.expires_at.is_some_and(|at| at <= 0) }",
        "fn f(c: Option<&C>) -> bool { c.and_then(|c| c.expires_at).is_some_and(|at| at <= 0) }",
        "fn f(c: &C) -> bool { matches!(c.expires_at, Some(at) if at < 0) }",
        "fn f(c: &C) -> bool { c.expires_at.map_or(false, |at| 0i64.ge(&at)) }",
        "fn f(c: &C) -> bool { let C { expires_at, .. } = c; expires_at.is_none() }",
        "fn f(c: &C) -> bool { if let C { expires_at: Some(at), .. } = c { *at < 0 } else { false } }",
        "fn f(c: &C) -> bool { c.r#expires_at.is_none() }",
        "fn f(c: &C) -> bool { let C { r#expires_at, .. } = c; r#expires_at.is_none() }",
        "fn f(c: &C) -> Option<i64> { field!(c, expires_at) }",
    ] {
        assert_eq!(expiry_reads(read), 1, "not counted: {read}");
    }
    for build in [
        "fn f() -> C { C { expires_at: None } }",
        "fn f() { let _ = format!(\"expires_at\"); }",
    ] {
        assert_eq!(expiry_reads(build), 0, "miscounted: {build}");
    }
}
