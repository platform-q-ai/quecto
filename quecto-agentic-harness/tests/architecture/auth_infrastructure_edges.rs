//! `infrastructure/auth` depends on no provider code (#1839 A3 review):
//! the providers' refresh decorator names auth's `RefreshFn`, never the
//! reverse, so the edge runs one way (providers → auth), as L22 (#2580)
//! expects. Exact: the infrastructure modules an auth adapter may name are
//! listed, and anything else — `providers` first of all, the layer itself,
//! or an alias the scan cannot follow — fails.
use std::path::Path;

/// The infrastructure modules `infrastructure/auth` may name, as paths
/// under `crate::infrastructure`: itself, the clock, the shared runtime
/// builder and the atomic file write. Anything else is refused.
const AUTH_MAY_NAME: &[&[&str]] = &[
    &["auth"],
    &["time"],
    &["processes", "tokio_runtime"],
    &["persistence", "file", "atomic_write"],
];

/// The `infrastructure` siblings `file`'s production code names that are
/// not allowed, relative paths resolved.
fn refused_siblings(file: &str, source: &str) -> Vec<String> {
    let paths = super::dependency_paths_at(file, source).unwrap_or_else(|| {
        panic!("{file} does not parse");
    });
    paths
        .into_iter()
        .filter(|path| {
            // An alias (`use crate::infrastructure as infra`) cannot be
            // followed, so it is refused rather than trusted.
            if path == super::dependency_scan::UNRESOLVABLE {
                return true;
            }
            let parts: Vec<&str> = path.split("::").collect();
            match parts.as_slice() {
                ["crate", "infrastructure", rest @ ..] if !rest.is_empty() => !AUTH_MAY_NAME
                    .iter()
                    .any(|allowed| rest.starts_with(allowed)),
                // The layer itself (`use crate::infrastructure;`,
                // `use super::super;`) would reach any sibling through it.
                ["crate", "infrastructure"] => true,
                _ => false,
            }
        })
        .collect()
}

#[test]
fn auth_infrastructure_names_no_provider_code() {
    let mut files = Vec::new();
    super::collect_rs_files(Path::new("src/infrastructure/auth"), &mut files);
    assert!(
        files.len() >= 8,
        "the auth adapters were read ({})",
        files.len()
    );
    let refused: Vec<String> = files
        .iter()
        .filter_map(|entry| entry.split_once(":\n"))
        .filter(|(file, _)| !super::dependency_scan::test_only_file(file))
        .flat_map(|(file, source)| {
            refused_siblings(file, source)
                .into_iter()
                .map(move |path| format!("{file}: {path}"))
        })
        .collect();
    assert!(
        refused.is_empty(),
        "infrastructure/auth may name only {AUTH_MAY_NAME:?}: {refused:#?}"
    );
}

#[test]
fn the_edge_check_refuses_the_reverse_edge_however_spelled() {
    let file = "src/infrastructure/auth/refresh_wiring.rs";
    for source in [
        "use crate::infrastructure::providers::routing::refreshable::RefreshableProvider;",
        "fn f() -> crate::infrastructure::providers::routing::refreshable::ProviderFactory { todo() }",
        "use super::super::providers::refreshable::RefreshableProvider;",
        "use crate::infrastructure::{auth::oauth, providers::router::Router};",
        "use crate::infrastructure; fn f() { let _ = infrastructure::providers::routing::router::x(); }",
        "use super::super; fn f() { let _ = super::providers::x(); }",
        "use crate::infrastructure as infra;",
        "use super::super as up;",
        "use crate as root;",
    ] {
        assert!(
            !refused_siblings(file, source).is_empty(),
            "not refused: {source}"
        );
    }
    assert!(
        refused_siblings(
            file,
            "use crate::infrastructure::time::unix_timestamp_secs;"
        )
        .is_empty()
    );
}
