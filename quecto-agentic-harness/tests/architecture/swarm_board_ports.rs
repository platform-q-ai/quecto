//! The board's tool adapter (#2303 review H2, round-3 review L5) reaches
//! the store's meter and the event log only through application ports:
//! every crate path its production files name — in a `use`, in any path of
//! the syntax tree (a type, an expression, a pattern, a trait bound) or in
//! a macro's tokens — is on the [`ALLOWED`] list, with `super::`/`self::`
//! resolved against the file's place in the module tree. A fully-qualified
//! `crate::infrastructure::…` anywhere in the code fails, not only on a
//! `use` line.
use super::dependency_scan;

/// The files checked: the dispatcher and its telemetry records.
const FILES: [&str; 2] = [
    "src/infrastructure/tools/swarm_board_dispatch.rs",
    "src/infrastructure/tools/swarm_board_telemetry.rs",
];

/// The crate modules those files may name: the application's and the
/// domain's, and the two files themselves (the dispatcher's test-only
/// module reaches its parent through `super::`).
const ALLOWED: &[&str] = &[
    "crate::application::",
    "crate::domain::",
    "crate::infrastructure::tools::swarm_board_dispatch::",
    "crate::infrastructure::tools::swarm_board_telemetry::",
];

fn allowed(path: &str) -> bool {
    ALLOWED.iter().any(|prefix| path.starts_with(prefix))
}

/// Every crate path `source` names, resolved from `module`.
fn crate_paths(source: &str, module: &[String]) -> Vec<String> {
    // Red stub (#2303 round-3 L5): the `use crate::` lines only.
    let _ = module;
    source
        .lines()
        .map(str::trim_start)
        .filter_map(|line| line.strip_prefix("use "))
        .filter(|line| line.starts_with("crate::"))
        .map(|line| line.trim_end_matches(';').to_owned())
        .collect()
}

#[test]
fn the_board_dispatcher_depends_only_on_ports() {
    for file in FILES {
        let source = std::fs::read_to_string(file).unwrap_or_else(|_| panic!("read {file}"));
        let module = dependency_scan::module_path(file);
        let paths = crate_paths(&source, &module);
        assert!(!paths.is_empty(), "{file} names the crate");
        let outside: Vec<_> = paths.iter().filter(|path| !allowed(path)).collect();
        assert!(outside.is_empty(), "{file} names {outside:?}");
    }
}

fn module(path: &str) -> Vec<String> {
    path.split("::").map(str::to_owned).collect()
}

#[test]
fn the_scan_finds_crate_paths_anywhere_in_the_code() {
    let here = module("infrastructure::tools::swarm_board_dispatch");
    let found = crate_paths(
        r#"
        use crate::application::swarm::ports::BoardOpLog;
        use crate::{domain::swarm::RefusalKind, infrastructure::persistence::A};
        use super::swarm_board_telemetry::trace;
        fn f(meter: crate::infrastructure::persistence::swarm_board::meter::Meter) {
            let _ = crate::infrastructure::persistence::audit_log::AuditLog::open();
            let _: Vec<super::super::persistence::B> = Vec::new();
            tracing::warn!("{}", crate::infrastructure::c());
            match x { crate::infrastructure::D::E => {} _ => {} }
        }
        mod test_only { use super::Served; }
        "#,
        &here,
    );
    for expected in [
        "crate::application::swarm::ports::BoardOpLog",
        "crate::domain::swarm::RefusalKind",
        "crate::infrastructure::persistence::A",
        "crate::infrastructure::tools::swarm_board_telemetry::trace",
        "crate::infrastructure::persistence::swarm_board::meter::Meter",
        "crate::infrastructure::persistence::audit_log::AuditLog::open",
        "crate::infrastructure::persistence::B",
        "crate::infrastructure::c",
        "crate::infrastructure::D::E",
    ] {
        assert!(
            found.iter().any(|path| path == expected),
            "{expected} in {found:?}"
        );
    }
    let outside: Vec<_> = found.iter().filter(|path| !allowed(path)).collect();
    assert_eq!(outside.len(), 6, "{outside:?}");
}

#[test]
fn a_crate_alias_is_never_allowed() {
    let here = module("infrastructure::tools::swarm_board_dispatch");
    let found = crate_paths("use crate as root; fn f() { root::x(); }", &here);
    assert!(
        found.iter().any(|path| !allowed(path)),
        "an alias of the crate cannot be followed: {found:?}"
    );
}
