//! The supervisor's one-thread runtime reaps every child and runs every
//! termination: no caller code runs on it except the termination
//! protocol (#1935; #2286 review rounds 2 to 4). Proven by syn over the
//! files that can see the supervisor's private fields and the files whose
//! code runs on its runtime:
//!
//! - its runtime fields are private, and only the allowlisted helpers
//!   reach them or spawn on any runtime, in any function anywhere
//!   ([`access`], which states the rules in full);
//! - no shape in those files carries code — no `dyn` object, type
//!   parameter, `impl Trait` parameter or fn pointer — but the pinned
//!   termination protocol's;
//! - no function of an impl on a type naming `OwnedChildSupervisor`
//!   (`Arc<OwnedChildSupervisor>` too, whatever its visibility) takes a
//!   closure, and only the allowlisted termination functions take a
//!   future: exactly the protocol, by name, type and bound;
//! - no type alias, field or other function's signature in those files
//!   names a closure or a future.
use std::path::Path;

use quote::ToTokens;
use syn::visit::Visit;

/// Who may reach the supervisor's runtime, proven over every function.
#[path = "supervisor_runtime_access.rs"]
mod access;
/// The checker rejects the bypasses review found (#2286 round 4).
#[path = "supervisor_runtime_access_tests.rs"]
mod access_tests;

const SUPERVISOR: &str = "src/infrastructure/processes/owned_child_supervisor.rs";

/// The only functions that may take a future: each with its file, the
/// visibility it must keep, and its protocol parameter's exact type and
/// the exact bound a generic protocol must have (empty for none).
const PROTOCOL_TAKERS: &[(&str, &str, &str, &str, &str)] = &[
    (
        SUPERVISOR,
        "terminate",
        "pub",
        "P",
        "P : Future < Output = ProtocolOutcome >",
    ),
    (SUPERVISOR, "request_termination", "pub", BOXED_PROTOCOL, ""),
    (
        TASKS,
        "request_termination_observed",
        "pub (crate)",
        BOXED_PROTOCOL,
        "",
    ),
    (SUPERVISOR, "spawn_termination", "", BOXED_PROTOCOL, ""),
];

const TASKS: &str = "src/infrastructure/processes/owned_child_supervisor/tasks.rs";

const PIPES: &str = "src/infrastructure/processes/child_line_pipes.rs";

const BOXED_PROTOCOL: &str =
    "std :: pin :: Pin < Box < dyn Future < Output = ProtocolOutcome > + Send > >";

/// Identifiers that name a closure.
const CLOSURE_WORDS: &[&str] = &["Fn", "FnMut", "FnOnce"];

/// Identifiers that name a future.
const FUTURE_WORDS: &[&str] = &["Future", "IntoFuture"];

fn tokens(item: &impl ToTokens) -> String {
    item.to_token_stream().to_string()
}

fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '_'))
}

fn names_any(text: &str, names: &[&str]) -> bool {
    words(text).any(|word| names.contains(&word))
}

/// One function of an `impl OwnedChildSupervisor`.
struct SupervisorFn {
    file: String,
    name: String,
    visibility: String,
    /// Each typed parameter: its pattern and its type.
    parameters: Vec<(String, String)>,
    /// Each generic parameter and where-clause predicate.
    bounds: Vec<String>,
}

/// Whether `ty` names the supervisor anywhere: `OwnedChildSupervisor`
/// itself, `Arc<OwnedChildSupervisor>`, a reference to either.
fn is_supervisor(ty: &syn::Type) -> bool {
    names_any(&tokens(ty), &["OwnedChildSupervisor"])
}

fn supervisor_fn(file: &str, method: &syn::ImplItemFn) -> SupervisorFn {
    let signature = &method.sig;
    let parameters = signature
        .inputs
        .iter()
        .filter_map(|input| match input {
            syn::FnArg::Typed(typed) => Some((tokens(&typed.pat), tokens(&typed.ty))),
            syn::FnArg::Receiver(_) => None,
        })
        .collect();
    let mut bounds: Vec<String> = signature.generics.params.iter().map(tokens).collect();
    if let Some(clause) = &signature.generics.where_clause {
        bounds.extend(clause.predicates.iter().map(tokens));
    }
    SupervisorFn {
        file: file.to_string(),
        name: signature.ident.to_string(),
        visibility: tokens(&method.vis),
        parameters,
        bounds,
    }
}

/// Every production file under `src/infrastructure/processes/`: its path
/// and its source.
fn process_sources() -> Vec<(String, String)> {
    let mut files = Vec::new();
    super::collect_rs_files(Path::new("src/infrastructure/processes"), &mut files);
    assert!(!files.is_empty(), "the processes module has files");
    files
        .iter()
        .map(|file| {
            let (path, source) = file.split_once(":\n").expect("a file entry");
            (path.to_string(), source.to_string())
        })
        .collect()
}

/// `sources`, parsed.
fn parse_sources(sources: &[(String, String)]) -> Vec<(String, syn::File)> {
    sources
        .iter()
        .map(|(path, source)| {
            let parsed = syn::parse_file(source).unwrap_or_else(|e| panic!("{path}: {e}"));
            (path.clone(), parsed)
        })
        .collect()
}

/// Every production file under `src/infrastructure/processes/`, parsed.
fn process_files() -> Vec<(String, syn::File)> {
    parse_sources(&process_sources())
}

/// Every function of every `impl OwnedChildSupervisor` (inherent or trait,
/// at any module depth) in `files`.
fn supervisor_fns(files: &[(String, syn::File)]) -> Vec<SupervisorFn> {
    struct Impls<'a> {
        file: &'a str,
        found: Vec<SupervisorFn>,
    }
    impl<'ast> Visit<'ast> for Impls<'_> {
        fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
            if is_supervisor(&item.self_ty) {
                for member in &item.items {
                    if let syn::ImplItem::Fn(method) = member {
                        self.found.push(supervisor_fn(self.file, method));
                    }
                }
            }
            syn::visit::visit_item_impl(self, item);
        }
    }
    let mut found = Vec::new();
    for (file, parsed) in files {
        let mut impls = Impls {
            file,
            found: Vec::new(),
        };
        impls.visit_file(parsed);
        found.extend(impls.found);
    }
    found
}

#[test]
fn the_supervisor_runtime_fields_are_private_and_reached_only_by_allowlisted_helpers() {
    let files = process_files();
    let (_, supervisor) = files
        .iter()
        .find(|(path, _)| path == SUPERVISOR)
        .expect("the supervisor's file");
    let fields = supervisor
        .items
        .iter()
        .find_map(|item| match item {
            syn::Item::Struct(item) if item.ident == "OwnedChildSupervisor" => Some(&item.fields),
            _ => None,
        })
        .expect("the supervisor struct");
    let runtime_fields: Vec<&syn::Field> = fields
        .iter()
        .filter(|field| names_any(&tokens(&field.ty), &["runtime"]))
        .collect();
    let names: Vec<String> = runtime_fields
        .iter()
        .filter_map(|field| field.ident.as_ref().map(ToString::to_string))
        .collect();
    assert_eq!(
        names,
        access::RUNTIME_FIELDS,
        "the supervisor's runtime fields are exactly the checked ones"
    );
    for field in runtime_fields {
        assert!(
            matches!(field.vis, syn::Visibility::Inherited),
            "the runtime field `{}` is private to {SUPERVISOR}'s module tree, not `{}`",
            tokens(&field.ident),
            tokens(&field.vis)
        );
    }
    let violations = access::violations(&files);
    assert!(
        violations.is_empty(),
        "the supervisor's runtime is reached outside its allowlisted helpers:\n{}",
        violations.join("\n")
    );
}

#[test]
fn no_supervisor_function_takes_a_closure_or_a_future_but_the_protocol() {
    let files = process_files();
    let mut protocols = Vec::new();
    for function in supervisor_fns(&files) {
        let at = format!("{}: `{}`", function.file, function.name);
        for (pattern, ty) in &function.parameters {
            assert!(
                !names_any(ty, CLOSURE_WORDS),
                "{at} takes a closure ({pattern}: {ty}): no caller code runs on the \
                 supervisor's runtime except the termination protocol (#1935)"
            );
        }
        for bound in &function.bounds {
            assert!(
                !names_any(bound, CLOSURE_WORDS),
                "{at} is generic over a closure ({bound})"
            );
        }
        let takes_future = function
            .parameters
            .iter()
            .map(|(_, ty)| ty)
            .chain(&function.bounds)
            .any(|text| names_any(text, FUTURE_WORDS));
        if !takes_future {
            continue;
        }
        let allowed = PROTOCOL_TAKERS
            .iter()
            .find(|(file, name, _, _, _)| *file == function.file && *name == function.name);
        let Some((_, name, visibility, protocol, bound)) = allowed else {
            panic!("{at} takes a future: only the PROTOCOL_TAKERS may, and only the protocol");
        };
        assert_eq!(
            function.visibility, *visibility,
            "{at} keeps its pinned visibility"
        );
        let futures: Vec<&(String, String)> = function
            .parameters
            .iter()
            .filter(|(_, ty)| names_any(ty, FUTURE_WORDS) || ty == protocol)
            .collect();
        assert_eq!(
            futures,
            [&("protocol".to_string(), protocol.to_string())],
            "{at}: the one future parameter is the protocol, of exactly its type"
        );
        let future_bounds: Vec<&String> = function
            .bounds
            .iter()
            .filter(|text| names_any(text, FUTURE_WORDS))
            .collect();
        let expected: Vec<&str> = [*bound].into_iter().filter(|b| !b.is_empty()).collect();
        assert_eq!(
            future_bounds, expected,
            "{at}: the only future bound is the protocol's, exactly"
        );
        protocols.push(*name);
    }
    for (_, name, _, _, _) in PROTOCOL_TAKERS {
        assert!(
            protocols.contains(name),
            "PROTOCOL_TAKERS lists `{name}`, which takes no protocol: remove it"
        );
    }
}

/// In the files whose code runs on the supervisor's runtime, no data shape
/// can carry code: no type alias, struct or enum field names a closure or
/// a future, and no function outside an `impl OwnedChildSupervisor` takes
/// one.
#[test]
fn no_runtime_file_declares_a_shape_that_carries_caller_code() {
    struct Shapes {
        found: Vec<String>,
    }
    impl<'ast> Visit<'ast> for Shapes {
        fn visit_item_type(&mut self, item: &'ast syn::ItemType) {
            self.found
                .push(format!("type {} = {}", item.ident, tokens(&item.ty)));
            syn::visit::visit_item_type(self, item);
        }
        fn visit_field(&mut self, field: &'ast syn::Field) {
            self.found.push(format!("field {}", tokens(field)));
            syn::visit::visit_field(self, field);
        }
        fn visit_signature(&mut self, signature: &'ast syn::Signature) {
            self.found.push(format!(
                "fn {}{} ({})",
                signature.ident,
                tokens(&signature.generics),
                tokens(&signature.inputs)
            ));
            syn::visit::visit_signature(self, signature);
        }
        fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
            // The supervisor's own functions are the other tests' (the
            // termination protocol is theirs to allow); its impls declare
            // no fields or aliases.
            if !is_supervisor(&item.self_ty) {
                syn::visit::visit_item_impl(self, item);
            }
        }
    }
    let files = process_files();
    for (path, parsed) in access::checked(&files) {
        let mut shapes = Shapes { found: Vec::new() };
        shapes.visit_file(parsed);
        assert!(!shapes.found.is_empty(), "{path} declares shapes");
        for shape in shapes.found {
            let carries_code = names_any(&shape, CLOSURE_WORDS) || names_any(&shape, FUTURE_WORDS);
            assert!(
                !carries_code,
                "{path}: `{shape}` can carry a caller's code onto the supervisor's runtime"
            );
        }
    }
}
