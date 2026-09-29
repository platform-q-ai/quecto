//! The board's tool adapter (#2303 review H2, round-3 review L5) reaches
//! the store's meter and the event log only through application ports:
//! every crate path its production files name — in a `use`, in any path of
//! the syntax tree (a type, an expression, a pattern, a trait bound) or in
//! a macro's tokens — is on the [`ALLOWED`] list, with `super::`/`self::`
//! resolved against the file's place in the module tree. A fully-qualified
//! `crate::infrastructure::…` anywhere in the code fails, not only on a
//! `use` line.
use syn::visit::Visit;

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

/// Every crate path `source` names, resolved from `module`: each leaf of
/// a `use` tree, each path of the syntax tree whose first segment is
/// `crate`, `super` or `self`, and each such path in a macro's tokens. A
/// crate alias (`use crate as c`) or an `extern crate` cannot be followed,
/// and is [`dependency_scan::UNRESOLVABLE`], which no allowlist admits.
fn crate_paths(source: &str, module: &[String]) -> Vec<String> {
    let file = syn::parse_file(source).expect("a crate source parses");
    let mut found = Paths {
        module: module.to_vec(),
        paths: Vec::new(),
    };
    found.visit_file(&file);
    found.paths
}

/// The crate paths found so far, and the module the visit is in (an inline
/// `mod` is entered, so its `super::` names the file's module).
struct Paths {
    module: Vec<String>,
    paths: Vec<String>,
}

impl Paths {
    /// Keeps `path` when it is crate-relative, resolved to `crate::…`.
    fn keep(&mut self, path: &str) {
        let first = path.split("::").next().unwrap_or_default();
        if matches!(first, "crate" | "super" | "self") {
            self.paths
                .push(dependency_scan::resolve_relative(&self.module, path));
        } else if path == dependency_scan::UNRESOLVABLE {
            self.paths.push(path.to_owned());
        }
    }
}

impl<'ast> Visit<'ast> for Paths {
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        self.module.push(item.ident.to_string());
        syn::visit::visit_item_mod(self, item);
        self.module.pop();
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        let mut leaves = Vec::new();
        dependency_scan::expand_use_tree(&item.tree, "", &mut leaves);
        for leaf in leaves {
            self.keep(&leaf);
        }
    }

    /// A visibility (`pub(super)`, `pub(in crate::x)`) names where an item
    /// is seen, not a dependency.
    fn visit_visibility(&mut self, _: &'ast syn::Visibility) {}

    fn visit_item_extern_crate(&mut self, _: &'ast syn::ItemExternCrate) {
        self.paths.push(dependency_scan::UNRESOLVABLE.to_owned());
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        // A lone `self` is a method's receiver (`match self`), not a path
        // into the crate; every crate path has two segments or more.
        if path.segments.len() < 2 {
            syn::visit::visit_path(self, path);
            return;
        }
        let text = path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>()
            .join("::");
        self.keep(&text);
        syn::visit::visit_path(self, path);
    }

    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        for path in dependency_scan::crate_paths_in_tokens(mac.tokens.clone()) {
            self.keep(&path);
        }
        syn::visit::visit_macro(self, mac);
    }
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
        mod test_only { use super::Served; pub(super) fn g() { let _ = super::super::x(); } }
        impl Method { fn name(self) -> u8 { match self { _ => 0 } } }
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
        "crate::infrastructure::tools::swarm_board_dispatch::Served",
        "crate::infrastructure::tools::x",
    ] {
        assert!(
            found.iter().any(|path| path == expected),
            "{expected} in {found:?}"
        );
    }
    let outside: Vec<_> = found.iter().filter(|path| !allowed(path)).collect();
    assert_eq!(outside.len(), 7, "{outside:?}");
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
