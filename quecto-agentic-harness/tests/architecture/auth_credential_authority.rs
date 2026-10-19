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
    // Transitional: `OAuthStoreParams.expires_at` in the browser login,
    // which #2591 (A2 of #1839) retires; drop this row with it.
    "src/interface/cli/auth.rs",
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

impl<'ast> Visit<'ast> for ExpiryReads {
    fn visit_expr_field(&mut self, field: &'ast syn::ExprField) {
        if matches!(&field.member, syn::Member::Named(name) if name == "expires_at") {
            self.0 += 1;
        }
        syn::visit::visit_expr_field(self, field);
    }

    fn visit_field_pat(&mut self, field: &'ast syn::FieldPat) {
        if matches!(&field.member, syn::Member::Named(name) if name == "expires_at") {
            self.0 += 1;
        }
        syn::visit::visit_field_pat(self, field);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        // A macro's tokens (`matches!(c.expires_at, …)`) are read as an
        // expression list when they parse as one.
        if let Ok(exprs) = mac.parse_body_with(
            syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated,
        ) {
            for expr in &exprs {
                self.visit_expr(expr);
            }
        } else if mac.tokens.to_string().contains("expires_at") {
            // Unparsable macro tokens naming the field count as a read.
            self.0 += 1;
        }
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
