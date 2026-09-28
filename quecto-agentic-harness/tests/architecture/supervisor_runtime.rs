//! The supervisor's one-thread runtime reaps every child and runs every
//! termination; it must not become a place to run a caller's work (#2286
//! review rounds 2 and 3). Proven by syn over every file of
//! `src/infrastructure/processes/`:
//!
//! - the runtime handle is a private field of `OwnedChildSupervisor`, and
//!   only the allowlisted functions of `owned_child_supervisor.rs` touch
//!   it — each at its pinned visibility, the pipe and termination spawn
//!   helpers private;
//! - no function of any `impl OwnedChildSupervisor` (whatever its
//!   visibility) takes a closure, and only the allowlisted termination
//!   functions take a future: exactly the termination protocol, by name,
//!   type and bound — mentioning `ProtocolOutcome` elsewhere passes
//!   nothing;
//! - in the files whose code runs on that runtime, no type alias, field or
//!   other function's signature names a closure or a future, so no data
//!   shape a caller passes in (a termination observation, a pipe task) can
//!   carry code.
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

/// The files whose code runs on the supervisor's runtime.
const RUNTIME_FILES: &[&str] = &[
    "src/infrastructure/processes/owned_child_supervisor.rs",
    "src/infrastructure/processes/owned_child_supervisor/tasks.rs",
    "src/infrastructure/processes/child_line_pipes.rs",
    "src/infrastructure/processes/child_stderr_tail.rs",
];

/// The functions of `owned_child_supervisor.rs` that touch the runtime
/// handle, each with the visibility it must keep, and why.
const RUNTIME_SPAWNERS: &[(&str, &str, &str)] = &[
    (
        "spawn",
        "pub",
        "spawns a Command on the runtime and adopts it",
    ),
    ("adopt", "", "the reap task of an adopted child"),
    (
        "retain_stderr_tail",
        "pub",
        "the stderr tail's drain: a pipe type in, bytes out",
    ),
    (
        "retain_stderr_tail_within",
        "pub",
        "the stderr tail's drain: a pipe type in, bytes out",
    ),
    (
        "retire_when_reaped",
        "pub",
        "waits for the reap, then retires",
    ),
    (
        "spawn_pipe_task",
        "",
        "the private spawn helper of the line pumps: a PipeTask, built only by child_line_pipes",
    ),
    (
        "spawn_termination",
        "",
        "the private spawn helper of every requested termination",
    ),
];

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
    touches_handle: bool,
}

#[derive(Default)]
struct HandleAccess(bool);

impl<'ast> Visit<'ast> for HandleAccess {
    fn visit_expr_field(&mut self, field: &'ast syn::ExprField) {
        if matches!(&field.member, syn::Member::Named(name) if name == "handle") {
            self.0 = true;
        }
        syn::visit::visit_expr_field(self, field);
    }
}

fn is_supervisor(ty: &syn::Type) -> bool {
    match ty {
        syn::Type::Path(path) => path
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "OwnedChildSupervisor"),
        _ => false,
    }
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
    let mut access = HandleAccess::default();
    access.visit_block(&method.block);
    SupervisorFn {
        file: file.to_string(),
        name: signature.ident.to_string(),
        visibility: tokens(&method.vis),
        parameters,
        bounds,
        touches_handle: access.0,
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
fn the_supervisor_runtime_handle_is_private_and_spawned_on_only_by_allowlisted_helpers() {
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
    let handle = fields
        .iter()
        .find(|field| field.ident.as_ref().is_some_and(|name| name == "handle"))
        .expect("the runtime handle field");
    assert!(
        matches!(handle.vis, syn::Visibility::Inherited),
        "the runtime handle is private to {SUPERVISOR}, not `{}`",
        tokens(&handle.vis)
    );
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
                 supervisor's runtime"
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
    for path in RUNTIME_FILES {
        let source = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
        let parsed = syn::parse_file(&source).unwrap_or_else(|e| panic!("{path}: {e}"));
        let mut shapes = Shapes { found: Vec::new() };
        shapes.visit_file(&parsed);
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
