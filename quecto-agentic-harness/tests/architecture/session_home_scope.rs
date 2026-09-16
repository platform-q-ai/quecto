//! Bounded architecture ratchets for folder-aware resume (#2001).
//!
//! The approved public request/response behavior is exercised through raw UDS
//! JSON by the issue BDD. This module deliberately does **not** prescribe a
//! `ResolveResume` Rust variant, request-field representation, new domain type,
//! port, adapter, token, catalogue format or error vocabulary. It ratchets only
//! responsibilities that are observable in the existing architecture:
//!
//! * the existing explicit-global list controller delegates its application
//!   query without moving storage/presentation ownership to the interface;
//! * the fieldless/default-local policy remains classified as raw-wire BDD
//!   until the public request can express it—this module does not relabel the
//!   existing `list_all` method as local;
//! * the existing resume edge delegates the transaction to the existing
//!   application owner rather than directly performing session-store effects;
//! * the application owner retains the ownership/load/save transaction;
//! * any interface implementation that appears later stays free of direct
//!   filesystem, process-launch and Git effects.
//!
//! Every check uses parsed executable Rust syntax. Comments and dead string
//! mentions cannot satisfy these contracts. Absence of a proposed new Rust
//! symbol is never itself a failure.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use syn::visit::{self, Visit};

const LIST_CONTROLLER: &str = "src/interface/uds/sessions/controller.rs";
const RESUME_EDGE: &str = "src/interface/cli/uds_dispatch_session.rs";
const RESUME_OWNER: &str = "src/application/sessions/use_cases/resume_saved_session.rs";
const INTERFACE_ROOT: &str = "src/interface";

fn parse(path: &Path) -> syn::File {
    let source = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("read architecture source {}: {error}", path.display()));
    syn::parse_file(&source)
        .unwrap_or_else(|error| panic!("parse architecture source {}: {error}", path.display()))
}

fn production_rs_files(root: &str) -> Vec<PathBuf> {
    fn visit(path: &Path, files: &mut Vec<PathBuf>) {
        if path.is_dir() {
            for entry in fs::read_dir(path).expect("read architecture directory") {
                visit(&entry.expect("read architecture entry").path(), files);
            }
        } else if path.extension().is_some_and(|extension| extension == "rs")
            && !path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with("_tests.rs"))
        {
            files.push(path.to_path_buf());
        }
    }
    let mut files = Vec::new();
    visit(Path::new(root), &mut files);
    files.sort();
    files
}

fn function(file: &syn::File, name: &str) -> syn::ItemFn {
    file.items
        .iter()
        .find_map(|item| match item {
            syn::Item::Fn(function) if function.sig.ident == name => Some(function.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("existing architecture owner `{name}` must remain executable"))
}

fn method(file: &syn::File, owner: &str, name: &str) -> syn::ImplItemFn {
    for item in &file.items {
        let syn::Item::Impl(implementation) = item else {
            continue;
        };
        let mut owner_facts = Facts::default();
        owner_facts.visit_type(&implementation.self_ty);
        if !owner_facts.identifiers.contains(owner) {
            continue;
        }
        for item in &implementation.items {
            if let syn::ImplItem::Fn(method) = item
                && method.sig.ident == name
            {
                return method.clone();
            }
        }
    }
    panic!("existing architecture owner `{owner}::{name}` must remain executable");
}

#[derive(Default)]
struct Facts {
    identifiers: BTreeSet<String>,
    method_calls: BTreeSet<String>,
    paths: Vec<Vec<String>>,
    execute_arguments: Vec<syn::Expr>,
}

impl<'ast> Visit<'ast> for Facts {
    fn visit_ident(&mut self, identifier: &'ast syn::Ident) {
        self.identifiers.insert(identifier.to_string());
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        self.method_calls.insert(call.method.to_string());
        if call.method == "execute" {
            self.execute_arguments.extend(call.args.iter().cloned());
        }
        visit::visit_expr_method_call(self, call);
    }

    fn visit_expr_path(&mut self, expression: &'ast syn::ExprPath) {
        self.paths.push(
            expression
                .path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect(),
        );
        visit::visit_expr_path(self, expression);
    }
}

fn function_facts(function: &syn::ItemFn) -> Facts {
    let mut facts = Facts::default();
    facts.visit_item_fn(function);
    facts
}

fn method_facts(method: &syn::ImplItemFn) -> Facts {
    let mut facts = Facts::default();
    facts.visit_impl_item_fn(method);
    facts
}

#[test]
fn explicit_global_list_controller_delegates_without_owning_effects() {
    let controller = parse(Path::new(LIST_CONTROLLER));
    let implementation = method(&controller, "ListSessionsController", "list_all");
    let facts = method_facts(&implementation);
    assert_eq!(
        facts.execute_arguments.len(),
        1,
        "explicit-global list controller delegates exactly one application query"
    );
    assert!(
        facts.method_calls.contains("execute"),
        "explicit-global list controller must delegate to the application owner"
    );
    let admitted_calls: BTreeSet<String> =
        BTreeSet::from(["execute".to_string(), "finish_non_exhaustive".to_string()]);
    let unowned: BTreeSet<_> = facts
        .method_calls
        .difference(&admitted_calls)
        .cloned()
        .collect();
    assert!(
        unowned.is_empty(),
        "list controller must not own persistence/presentation effects; unowned calls: {unowned:?}"
    );
}

#[test]
fn existing_resume_edge_delegates_instead_of_owning_store_effects() {
    let edge = function(&parse(Path::new(RESUME_EDGE)), "handle_resume_session");
    let facts = function_facts(&edge);

    assert!(
        facts.method_calls.contains("execute"),
        "resume_session interface edge must delegate the transaction inward"
    );
    let admitted_edge_calls: BTreeSet<String> = BTreeSet::from(
        [
            "as_ref",
            "as_deref",
            "clone",
            "execute",
            "is_streaming",
            "new",
            "send",
            "to_string",
        ]
        .map(str::to_string),
    );
    let unowned: BTreeSet<_> = facts
        .method_calls
        .difference(&admitted_edge_calls)
        .cloned()
        .collect();
    assert!(
        unowned.is_empty(),
        "#2001 resume edge is parse/map/present only; unowned method calls: {unowned:?}"
    );
}

#[test]
fn existing_application_owner_retains_the_resume_transaction() {
    let owner = parse(Path::new(RESUME_OWNER));
    let execute = method(&owner, "ResumeSavedSession", "execute");
    let facts = method_facts(&execute);

    // These are existing transaction responsibilities, not proposed new API
    // names. Their continued co-location prevents ownership/load/save ordering
    // from drifting into the UDS/TUI interface during #2001.
    for responsibility in ["claim", "load_claimed", "release", "save", "settle"] {
        assert!(
            facts.method_calls.contains(responsibility),
            "ResumeSavedSession::execute must retain existing `{responsibility}` responsibility"
        );
    }
}

#[derive(Default)]
struct ApprovedProtocolReferences {
    files: BTreeSet<PathBuf>,
    current_file: PathBuf,
}

impl<'ast> Visit<'ast> for ApprovedProtocolReferences {
    fn visit_lit_str(&mut self, literal: &'ast syn::LitStr) {
        // Executable string values only; doc comments are attributes and are
        // not visited as expression literals by this visitor.
        if literal.value() == "resolve_resume" {
            self.files.insert(self.current_file.clone());
        }
        visit::visit_lit_str(self, literal);
    }
}

#[test]
fn discovered_resolution_edges_do_not_own_outer_effects() {
    let mut references = ApprovedProtocolReferences::default();
    for path in production_rs_files(INTERFACE_ROOT) {
        references.current_file = path;
        references.visit_file(&parse(&references.current_file));
    }

    // Raw-wire BDD owns the RED for the approved resolve_resume command's
    // absence. Once an implementation exists, this ratchet automatically
    // applies without requiring any particular enum variant or Rust field.
    for path in references.files {
        let mut facts = Facts::default();
        facts.visit_file(&parse(&path));
        let outer_effects: BTreeSet<String> = BTreeSet::from(
            [
                "canonicalize",
                "chdir",
                "create_dir",
                "create_dir_all",
                "load",
                "save",
                "spawn",
            ]
            .map(str::to_string),
        );
        let violations: BTreeSet<_> = facts
            .method_calls
            .intersection(&outer_effects)
            .cloned()
            .collect();
        assert!(
            violations.is_empty(),
            "#2001 resolution interface must delegate filesystem/persistence/launch policy; \
             {} directly calls {violations:?}",
            path.display()
        );
    }
}

#[test]
fn architecture_fixture_targets_are_real() {
    for path in [LIST_CONTROLLER, RESUME_EDGE, RESUME_OWNER] {
        assert!(
            Path::new(path).is_file(),
            "architecture target must exist: {path}"
        );
    }
    assert!(
        Path::new(INTERFACE_ROOT).is_dir(),
        "interface root must exist"
    );
}
