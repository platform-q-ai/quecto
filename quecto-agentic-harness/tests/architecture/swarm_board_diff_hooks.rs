//! The differential harness's tamper hook (#2270 round-2 review L3):
//! `try_run_both(steps, after)` lets `after` rewrite the Rust side's file
//! or answer before the comparison, which only a harness self-test may do
//! and still pass. So every `try_run_both(` call under `tests/` either
//! passes the no-op hook (a closure whose body is an empty block), or sits
//! in a `harness_self_test_*` function, or is followed by `.unwrap_err()`
//! (a scenario that expects the two boards to differ). A call the syntax
//! walk cannot see (inside a macro's tokens) is itself refused.
use std::path::{Path, PathBuf};

use proc_macro2::{Delimiter, TokenStream, TokenTree};
use syn::visit::Visit;

/// The harness entry point whose hook is checked.
const HARNESS: &str = "try_run_both";
/// Functions that test the harness itself, and so may tamper and pass.
const SELF_TEST_PREFIX: &str = "harness_self_test_";

/// One `try_run_both` call the rule refuses.
#[derive(Debug, PartialEq, Eq)]
struct Violation {
    function: Option<String>,
    line: usize,
}

#[derive(Default)]
struct Calls {
    function: Option<String>,
    seen: usize,
    violations: Vec<Violation>,
}

/// `try_run_both`, by any path ending in that name.
fn names_harness(func: &syn::Expr) -> bool {
    matches!(
        func,
        syn::Expr::Path(path)
            if path.path.segments.last().is_some_and(|segment| segment.ident == HARNESS)
    )
}

/// `try_run_both(...)`.
fn harness_call(expr: &syn::Expr) -> Option<&syn::ExprCall> {
    match expr {
        syn::Expr::Call(call) if names_harness(&call.func) => Some(call),
        _ => None,
    }
}

/// The no-op hook: a closure whose body is an empty block.
fn no_op(hook: &syn::Expr) -> bool {
    match hook {
        syn::Expr::Closure(closure) => matches!(
            &*closure.body,
            syn::Expr::Block(block) if block.block.stmts.is_empty()
        ),
        _ => false,
    }
}

impl Calls {
    fn in_function(&mut self, name: String, walk: impl FnOnce(&mut Self)) {
        let outer = self.function.replace(name);
        walk(self);
        self.function = outer;
    }

    /// One harness call; `expecting_difference` when `.unwrap_err()`
    /// follows it.
    fn check(&mut self, call: &syn::ExprCall, expecting_difference: bool) {
        self.seen += 1;
        let permitted = call.args.iter().nth(1).is_some_and(no_op)
            || expecting_difference
            || self
                .function
                .as_deref()
                .is_some_and(|name| name.starts_with(SELF_TEST_PREFIX));
        if !permitted {
            self.violations.push(Violation {
                function: self.function.clone(),
                line: syn::spanned::Spanned::span(&call.func).start().line,
            });
        }
        for argument in &call.args {
            self.visit_expr(argument);
        }
    }
}

impl<'ast> Visit<'ast> for Calls {
    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        self.in_function(item.sig.ident.to_string(), |calls| {
            syn::visit::visit_item_fn(calls, item);
        });
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        self.in_function(item.sig.ident.to_string(), |calls| {
            syn::visit::visit_impl_item_fn(calls, item);
        });
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        match harness_call(&call.receiver) {
            Some(harness) if call.method == "unwrap_err" => {
                self.check(harness, true);
                for argument in &call.args {
                    self.visit_expr(argument);
                }
            }
            _ => syn::visit::visit_expr_method_call(self, call),
        }
    }

    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        if names_harness(&call.func) {
            self.check(call, false);
        } else {
            syn::visit::visit_expr_call(self, call);
        }
    }
}

/// Every `try_run_both(` in `tokens`, macro bodies included and the
/// function's own definition (`fn try_run_both(`) excluded.
fn token_calls(tokens: TokenStream) -> usize {
    let trees: Vec<TokenTree> = tokens.into_iter().collect();
    let mut count = 0;
    for (index, tree) in trees.iter().enumerate() {
        match tree {
            TokenTree::Ident(ident) if ident == HARNESS => {
                let defined = index
                    .checked_sub(1)
                    .and_then(|before| trees.get(before))
                    .is_some_and(|before| matches!(before, TokenTree::Ident(word) if word == "fn"));
                let called = matches!(
                    trees.get(index + 1),
                    Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Parenthesis
                );
                count += usize::from(called && !defined);
            }
            TokenTree::Group(group) => count += token_calls(group.stream()),
            _ => {}
        }
    }
    count
}

/// The violations in one file's `source`; a call only the token scan
/// finds (inside a macro) is a violation of its own.
fn violations(source: &str) -> Vec<String> {
    let file = syn::parse_file(source).expect("the test file parses");
    let mut calls = Calls::default();
    calls.visit_file(&file);
    let mut found: Vec<String> = calls
        .violations
        .iter()
        .map(|violation| {
            format!(
                "line {} in {}: a tampering hook outside a harness self-test, with no .unwrap_err()",
                violation.line,
                violation.function.as_deref().unwrap_or("<no function>")
            )
        })
        .collect();
    let tokens: TokenStream = source.parse().expect("the test file tokenizes");
    let all = token_calls(tokens);
    if all != calls.seen {
        found.push(format!(
            "{} {HARNESS} call(s) the syntax walk cannot check (inside a macro)",
            all.abs_diff(calls.seen)
        ));
    }
    found
}

fn rust_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|error| panic!("read {}: {error}", dir.display()));
    for entry in entries {
        let path = entry.expect("a directory entry").path();
        if path.is_dir() {
            rust_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

#[test]
fn differential_hooks_tamper_only_in_self_tests_or_expected_differences() {
    let mut files = Vec::new();
    rust_files(Path::new("tests"), &mut files);
    files.sort();
    let mut failures = Vec::new();
    let mut calling_files = 0;
    for path in files {
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        if source.contains(HARNESS) {
            calling_files += 1;
            failures.extend(
                violations(&source)
                    .into_iter()
                    .map(|failure| format!("{}: {failure}", path.display())),
            );
        }
    }
    assert!(
        calling_files >= 3,
        "the scan finds the harness and its callers ({calling_files} files)"
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The checker itself: each permitted shape passes, and a tampering hook
/// in an ordinary test, or a call hidden in a macro, is caught.
#[test]
fn the_hook_checker_rejects_tampering_outside_self_tests() {
    let permitted = r#"
        pub fn try_run_both(steps: &[Step], after: impl FnMut()) -> Result<(), String> { Ok(()) }
        fn run_both(steps: &[Step]) {
            if let Err(difference) = try_run_both(steps, |_, _, _| {}) { panic!("{difference}"); }
        }
        #[test]
        fn harness_self_test_tampers() {
            let difference = try_run_both(&steps, |index, rust, _| { tamper(rust) });
        }
        #[test]
        fn expects_a_difference() {
            let difference = scenario::try_run_both(&steps, |_, rust, _| { tamper(rust) }).unwrap_err();
        }
    "#;
    assert_eq!(violations(permitted), Vec::<String>::new());

    let ordinary = r#"
        #[test]
        fn a_scenario() {
            try_run_both(&steps, |_, rust, _| { tamper(rust) }).unwrap();
        }
    "#;
    let found = violations(ordinary);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("in a_scenario"), "{found:?}");

    // `.unwrap_err()` on something else does not excuse the call inside.
    let wrapped = r#"
        fn a_scenario() {
            check(try_run_both(&steps, |_, rust, _| { tamper(rust) })).unwrap_err();
        }
    "#;
    assert_eq!(violations(wrapped).len(), 1);

    let hidden = r#"
        fn a_scenario() {
            assert!(try_run_both(&steps, |_, rust, _| { tamper(rust) }).is_ok());
        }
    "#;
    let found = violations(hidden);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("inside a macro"), "{found:?}");
}
