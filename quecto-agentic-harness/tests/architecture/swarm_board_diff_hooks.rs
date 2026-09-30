//! The differential harness's tamper hook (#2270 round-2 review L3,
//! round-3 L3/L4): `try_run_golden(steps, after)` lets `after` rewrite the
//! Rust side's file or answer before the comparison, which only a harness
//! self-test may do and still pass, and its `Err` is a difference between
//! the boards, which only a pinned divergence may expect. So every
//! `try_run_golden` under `tests/`:
//!
//! - sits in a `harness_self_test_*` function (any hook); or
//! - passes the no-op hook (a closure whose body is an empty block) and
//!   either sits in one of the pin table's own tests, checking the `Err`
//!   (the receiver of `.unwrap_err()` or `.expect_err(..)`), or in the
//!   harness's own `run_golden`, without either (round-4 L2). A pin table's
//!   own test is a top-level `#[test]` function of a file the pin table
//!   lists in `PIN_TABLE_FILES` (`swarm_board_diff_loose.rs` itself, and
//!   each file holding a pin of its own, #2275; a file is a pin table
//!   only by being listed, PR #2321 final review), named by a divergence
//!   of `PERMITTED_DIVERGENCES` not pinned elsewhere (`EXTERNAL_PINS`
//!   names neither it nor its test), or by the second pin `SECOND_PINS`
//!   gives such a divergence (#2273's `outside_edited_evidence` pin in
//!   `swarm_board_diff_submissions.rs`); and
//!   the harness's `run_golden` is the top-level function of that name in
//!   the harness's file.
//!
//! The harness is reached by its own name and called directly: an import
//! renaming it (`use …::try_run_golden as t;`) is refused, and so is any
//! `try_run_golden` token the syntax walk does not see as a direct call or a
//! plain import (a use as a value, or a call inside a macro's tokens).
use std::path::{Path, PathBuf};

use proc_macro2::{TokenStream, TokenTree};
use syn::visit::Visit;

/// The harness entry point whose hook is checked.
const HARNESS: &str = "try_run_golden";
/// The harness's own caller, which compares without expecting a difference.
const HARNESS_RUNNER: &str = "run_golden";
/// Functions that test the harness itself, and so may tamper and pass.
const SELF_TEST_PREFIX: &str = "harness_self_test_";
/// The file holding the pin table.
const PIN_TABLE: &str = "tests/integration/swarm_board_diff_loose.rs";
/// The folder the pin table's `include_str!` paths are relative to.
const PIN_TABLE_DIR: &str = "tests/integration/";
/// The pin table's constant listing, by `include_str!`, the files that
/// hold its pins: itself and every other file holding one (#2275). The one
/// source of truth for which files may expect a difference (PR #2321 final
/// review).
const PIN_TABLE_FILES: &str = "PIN_TABLE_FILES";
/// The pin table's constant naming, for a divergence it pins, a second
/// pinning test of another name: `(divergence, test)` pairs.
const SECOND_PINS: &str = "SECOND_PINS";
/// The file defining the harness and its `run_golden`.
const HARNESS_FILE: &str = "tests/common/swarm_board_diff/scenario.rs";
/// The file holding `outside_edited_evidence`'s second pin, listed in
/// `PIN_TABLE_FILES`, and that pin's test.
const EVIDENCE_PIN_FILE: &str = "tests/integration/swarm_board_diff_submissions.rs";
const EVIDENCE_PIN_TEST: &str = "edited_string_evidence_raises_in_python_and_is_refused_in_rust";
/// The golden self-test outside the `harness_self_test_` prefix (its
/// name is #2283's): `(file, function)`.
const GOLDEN_SELF_TEST: (&str, &str) = (
    "tests/integration/swarm_board_diff_runs.rs",
    "golden_replay_detects_a_changed_rust_result",
);
/// The methods that check a harness call's `Err`.
const EXPECTING_DIFFERENCE: [&str; 2] = ["unwrap_err", "expect_err"];
/// A test file that is neither.
const ELSEWHERE: &str = "tests/integration/a_scenario.rs";
/// The pin table's constants: the divergences, and those pinned outside
/// the suite (every string literal in it names one or its pinning test).
const PERMITTED: &str = "PERMITTED_DIVERGENCES";
const EXTERNAL: &str = "EXTERNAL_PINS";

/// One harness use the rule refuses.
#[derive(Debug, PartialEq, Eq)]
struct Violation {
    function: Option<String>,
    line: usize,
    problem: &'static str,
}

/// The string literals of one constant's value.
#[derive(Default)]
struct Literals(Vec<String>);

impl<'ast> Visit<'ast> for Literals {
    fn visit_lit_str(&mut self, literal: &'ast syn::LitStr) {
        self.0.push(literal.value());
    }
}

/// The value of the pin table's constant `name`.
fn constant<'file>(file: &'file syn::File, name: &str) -> &'file syn::Expr {
    let found: Vec<&syn::Expr> = file
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Const(constant) if constant.ident == name => Some(&*constant.expr),
            _ => None,
        })
        .collect();
    assert_eq!(found.len(), 1, "{PIN_TABLE} defines {name} once");
    found[0]
}

/// The string literals of the pin table's constant `name`.
fn constant_literals(file: &syn::File, name: &str) -> Vec<String> {
    let mut literals = Literals::default();
    literals.visit_expr(constant(file, name));
    literals.0
}

/// The pin table's file, parsed.
fn pin_table() -> syn::File {
    let source = std::fs::read_to_string(PIN_TABLE)
        .unwrap_or_else(|error| panic!("read {PIN_TABLE}: {error}"));
    syn::parse_file(&source).expect("the pin table parses")
}

/// The files the pin table lists in `PIN_TABLE_FILES`, as paths relative
/// to the crate: each entry must be `include_str!("<file>.rs")` naming a
/// file beside the pin table, and the pin table must list itself.
fn pin_table_files() -> Vec<String> {
    let file = pin_table();
    let syn::Expr::Array(array) = constant(&file, PIN_TABLE_FILES) else {
        panic!("{PIN_TABLE_FILES} is an array of include_str! calls");
    };
    let listed: Vec<String> = array
        .elems
        .iter()
        .map(|element| {
            let name = match element {
                syn::Expr::Macro(call) if call.mac.path.is_ident("include_str") => call
                    .mac
                    .parse_body::<syn::LitStr>()
                    .expect("include_str! takes one string literal")
                    .value(),
                _ => panic!("{PIN_TABLE_FILES} holds only include_str! calls"),
            };
            let plain = name.ends_with(".rs")
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.');
            assert!(
                plain,
                "{PIN_TABLE_FILES} names a file beside the pin table: {name}"
            );
            let path = format!("{PIN_TABLE_DIR}{name}");
            assert!(Path::new(&path).is_file(), "{PIN_TABLE_FILES} names {path}");
            path
        })
        .collect();
    assert!(
        listed.iter().any(|path| path == PIN_TABLE),
        "{PIN_TABLE_FILES} lists the pin table itself"
    );
    listed
}

/// The `(divergence, test)` pairs of the pin table's `SECOND_PINS`: an
/// array of two-string tuples.
fn second_pins(file: &syn::File) -> Vec<(String, String)> {
    let syn::Expr::Array(array) = constant(file, SECOND_PINS) else {
        panic!("{SECOND_PINS} is an array of (divergence, test) pairs");
    };
    array
        .elems
        .iter()
        .map(|element| {
            let mut literals = Literals::default();
            literals.visit_expr(element);
            match (element, literals.0.as_slice()) {
                (syn::Expr::Tuple(_), [divergence, test]) => (divergence.clone(), test.clone()),
                _ => panic!("{SECOND_PINS} holds only (divergence, test) pairs"),
            }
        })
        .collect()
}

/// The test names the pin table allows to expect a difference: the
/// divergences it pins itself, so none `EXTERNAL_PINS` names, and the
/// second pins `SECOND_PINS` gives them.
fn pinned_tests() -> Vec<String> {
    let file = pin_table();
    let external = constant_literals(&file, EXTERNAL);
    assert!(!external.is_empty(), "{EXTERNAL} names its pins");
    let mut pinned: Vec<String> = constant_literals(&file, PERMITTED)
        .into_iter()
        .filter(|name| !external.contains(name))
        .collect();
    assert!(!pinned.is_empty(), "the pin table pins divergences itself");
    for (divergence, test) in second_pins(&file) {
        assert!(
            pinned.contains(&divergence),
            "{SECOND_PINS}: {test} pins {divergence}, which the pin table does not pin itself"
        );
        pinned.push(test);
    }
    pinned
}

/// What the file being checked is to the harness.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    PinTable,
    Harness,
    Other,
}

impl Role {
    /// A pin table only when `pin_tables` (the pin table's
    /// `PIN_TABLE_FILES`) lists `path` exactly.
    fn of(path: &str, pin_tables: &[String]) -> Self {
        match path {
            HARNESS_FILE => Self::Harness,
            _ if pin_tables.iter().any(|listed| listed == path) => Self::PinTable,
            _ => Self::Other,
        }
    }
}

/// The function a harness call sits in.
#[derive(Clone)]
struct Function {
    name: String,
    /// One of the pin table's own tests.
    pinned: bool,
    /// The harness's own `run_golden`.
    runner: bool,
}

struct Calls {
    pinned: Vec<String>,
    role: Role,
    function: Option<Function>,
    seen: usize,
    violations: Vec<Violation>,
}

/// `try_run_golden`, by any path ending in that name.
fn names_harness(func: &syn::Expr) -> bool {
    matches!(
        func,
        syn::Expr::Path(path)
            if path.path.segments.last().is_some_and(|segment| segment.ident == HARNESS)
    )
}

/// `try_run_golden(...)`.
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
    fn new(pinned: Vec<String>, role: Role) -> Self {
        Self {
            pinned,
            role,
            function: None,
            seen: 0,
            violations: Vec::new(),
        }
    }

    /// Walks a function's body as the function `name`: only a top-level
    /// function is one of the pin table's tests or the harness's runner.
    fn in_function(&mut self, name: String, test: bool, walk: impl FnOnce(&mut Self)) {
        let top_level = self.function.is_none();
        let function = Function {
            pinned: top_level
                && test
                && match self.role {
                    Role::PinTable => self.pinned.contains(&name),
                    Role::Harness | Role::Other => false,
                },
            runner: top_level && self.role == Role::Harness && name == HARNESS_RUNNER,
            name,
        };
        let outer = self.function.replace(function);
        walk(self);
        self.function = outer;
    }

    fn refuse(&mut self, span: proc_macro2::Span, problem: &'static str) {
        self.violations.push(Violation {
            function: self.function.as_ref().map(|function| function.name.clone()),
            line: span.start().line,
            problem,
        });
    }

    /// The problem with one harness call, if any; `expecting_difference`
    /// when `.unwrap_err()` or `.expect_err(..)` follows it.
    fn problem(&self, call: &syn::ExprCall, expecting_difference: bool) -> Option<&'static str> {
        let function = self.function.as_ref();
        let self_test =
            function.is_some_and(|function| function.name.starts_with(SELF_TEST_PREFIX));
        let pinned = function.is_some_and(|function| function.pinned) && expecting_difference;
        let runner = function.is_some_and(|function| function.runner) && !expecting_difference;
        let quiet = call.args.iter().nth(1).is_some_and(no_op);
        match (self_test, quiet, pinned || runner) {
            (true, _, _) | (false, true, true) => None,
            (false, false, _) => Some("a tampering hook outside a harness self-test"),
            (false, true, false) => Some(
                "a harness call outside a pinned divergence test that unwraps its Err (only the harness's run_golden compares without expecting a difference)",
            ),
        }
    }

    fn check(&mut self, call: &syn::ExprCall, expecting_difference: bool) {
        self.seen += 1;
        if let Some(problem) = self.problem(call, expecting_difference) {
            self.refuse(syn::spanned::Spanned::span(&call.func), problem);
        }
        for argument in &call.args {
            self.visit_expr(argument);
        }
    }
}

impl<'ast> Visit<'ast> for Calls {
    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        let test = item
            .attrs
            .iter()
            .any(|attribute| attribute.path().is_ident("test"));
        self.in_function(item.sig.ident.to_string(), test, |calls| {
            syn::visit::visit_item_fn(calls, item);
        });
    }

    fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
        self.in_function(item.sig.ident.to_string(), false, |calls| {
            syn::visit::visit_impl_item_fn(calls, item);
        });
    }

    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        match harness_call(&call.receiver) {
            Some(harness)
                if EXPECTING_DIFFERENCE
                    .iter()
                    .any(|method| call.method == method) =>
            {
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

    /// A plain import of the harness is a use the walk has seen.
    fn visit_use_name(&mut self, name: &'ast syn::UseName) {
        if name.ident == HARNESS {
            self.seen += 1;
        }
    }

    /// An import renaming the harness hides its calls from the walk.
    fn visit_use_rename(&mut self, rename: &'ast syn::UseRename) {
        if rename.ident == HARNESS {
            self.seen += 1 + usize::from(rename.rename == HARNESS);
            self.refuse(rename.ident.span(), "an import renames try_run_golden");
        }
    }
}

/// Every `try_run_golden` token in `tokens`, macro bodies included and a
/// function's own definition (`fn try_run_golden`) excluded.
fn harness_tokens(tokens: TokenStream) -> usize {
    let trees: Vec<TokenTree> = tokens.into_iter().collect();
    let mut count = 0;
    for (index, tree) in trees.iter().enumerate() {
        match tree {
            TokenTree::Ident(ident) if ident == HARNESS => {
                let defined = index
                    .checked_sub(1)
                    .and_then(|before| trees.get(before))
                    .is_some_and(|before| matches!(before, TokenTree::Ident(word) if word == "fn"));
                count += usize::from(!defined);
            }
            TokenTree::Group(group) => count += harness_tokens(group.stream()),
            _ => {}
        }
    }
    count
}

/// The violations in the `source` of the file at `path` (relative to the
/// crate); a harness token the syntax walk did not see as a direct call or
/// a plain import is a violation of its own.
fn violations(path: &str, source: &str) -> Vec<String> {
    let file = syn::parse_file(source).expect("the test file parses");
    let mut calls = Calls::new(pinned_tests(), Role::of(path, &pin_table_files()));
    calls.visit_file(&file);
    let mut found: Vec<String> = calls
        .violations
        .iter()
        .map(|violation| {
            format!(
                "line {} in {}: {}",
                violation.line,
                violation.function.as_deref().unwrap_or("<no function>"),
                violation.problem
            )
        })
        .collect();
    let tokens: TokenStream = source.parse().expect("the test file tokenizes");
    let all = harness_tokens(tokens);
    if all != calls.seen {
        found.push(format!(
            "{} {HARNESS} use(s) the syntax walk cannot check (not called directly, or inside a macro)",
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
                violations(&path.to_string_lossy(), &source)
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
    let harness = r#"
        pub fn try_run_golden(steps: &[Step], after: impl FnMut()) -> Result<(), String> { Ok(()) }
        fn run_golden(steps: &[Step]) {
            if let Err(difference) = try_run_golden(steps, |_, _, _| {}) { panic!("{difference}"); }
        }
    "#;
    assert_eq!(violations(HARNESS_FILE, harness), Vec::<String>::new());
    let self_test = r#"
        #[test]
        fn harness_self_test_tampers() {
            let difference = try_run_golden(&steps, |index, rust, _| { tamper(rust) });
        }
    "#;
    assert_eq!(violations(ELSEWHERE, self_test), Vec::<String>::new());
    let pinned = r#"
        #[test]
        fn outside_edited_columns() {
            let difference = scenario::try_run_golden(&steps, |_, _, _| {}).unwrap_err();
        }
    "#;
    assert_eq!(violations(PIN_TABLE, pinned), Vec::<String>::new());
    // A second pin, in the listed file holding it.
    let evidence_pin = pinned.replace("outside_edited_columns", EVIDENCE_PIN_TEST);
    let found = violations(EVIDENCE_PIN_FILE, &evidence_pin);
    assert_eq!(found, Vec::<String>::new());
    // Only there: a file the pin table does not list holds no pin.
    assert_eq!(violations(ELSEWHERE, &evidence_pin).len(), 1);

    let ordinary = r#"
        #[test]
        fn a_scenario() {
            try_run_golden(&steps, |_, rust, _| { tamper(rust) }).unwrap();
        }
    "#;
    let found = violations(ELSEWHERE, ordinary);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("in a_scenario"), "{found:?}");

    // `.unwrap_err()` on something else does not excuse the call inside.
    let wrapped = r#"
        fn a_scenario() {
            check(try_run_golden(&steps, |_, rust, _| { tamper(rust) })).unwrap_err();
        }
    "#;
    assert_eq!(violations(ELSEWHERE, wrapped).len(), 1);

    let hidden = r#"
        fn a_scenario() {
            assert!(try_run_golden(&steps, |_, rust, _| { tamper(rust) }).is_ok());
        }
    "#;
    let found = violations(ELSEWHERE, hidden);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("inside a macro"), "{found:?}");
}

/// An expected difference is a pinned divergence (#2270 round-3 review
/// L3): outside a harness self-test, only the harness's own `run_golden` and
/// the pin table's own tests (named in `PERMITTED_DIVERGENCES`, pinned in
/// that file) may call `try_run_golden`, and only with the no-op hook.
#[test]
fn the_hook_checker_requires_expected_differences_to_be_pinned() {
    let pinned = r#"
        #[test]
        fn outside_edited_columns() {
            let difference = try_run_golden(&steps, |_, _, _| {}).unwrap_err();
        }
        #[test]
        fn integer_beyond_i64_is_refused() {
            for args in cases {
                let difference = try_run_golden(&steps, |_, _, _| {}).expect_err("a difference");
            }
        }
    "#;
    assert_eq!(violations(PIN_TABLE, pinned), Vec::<String>::new());
    // A sibling of the pin table may hold a slice's pins (#2275) when the
    // pin table lists it in `PIN_TABLE_FILES`; a file merely named like
    // one, below a folder so named, or unlisted (PR #2321 final review:
    // `_probe.rs`, and `_runs.rs` and `_completion.rs`, which hold only
    // pins of `EXTERNAL_PINS`) may not.
    let sibling = "tests/integration/swarm_board_diff_loose_files.rs";
    assert_eq!(violations(sibling, pinned), Vec::<String>::new());
    for elsewhere in [
        "tests/integration/swarm_board_diff_loose_files.txt",
        "tests/integration/swarm_board_diff_loose_x/files.rs",
        "tests/integration/swarm_board_diff_loosely.rs",
        "tests/integration/swarm_board_diff_loose_probe.rs",
        "tests/integration/swarm_board_diff_loose_runs.rs",
        "tests/integration/swarm_board_diff_loose_completion.rs",
    ] {
        assert_eq!(violations(elsewhere, pinned).len(), 2, "{elsewhere}");
    }

    for unpinned in [
        "fn a_scenario() { let d = try_run_golden(&steps, |_, _, _| {}).unwrap_err(); }",
        "fn a_scenario() { let held = try_run_golden(&steps, |_, _, _| {}); let d = held.unwrap_err(); }",
        "fn a_scenario() { if let Err(d) = try_run_golden(&steps, |_, _, _| {}) { check(d); } }",
        // `run_golden` is the harness's, and never expects a difference.
        "fn run_golden(steps: &[Step]) { try_run_golden(steps, |_, _, _| {}).unwrap_err(); }",
    ] {
        let found = violations(PIN_TABLE, unpinned);
        assert_eq!(found.len(), 1, "{unpinned}: {found:?}");
        assert!(
            found[0].contains("outside a pinned divergence test"),
            "{unpinned}: {found:?}"
        );
    }

    // Another test in the focused file is not licensed to expect a difference.
    let ordinary_in_pin_file = r#"
        #[test]
        fn edited_stale_first_malformed_later_evidence_is_refused_identically() {
            try_run_golden(&steps, |_, _, _| {}).unwrap_err();
        }
    "#;
    let found = violations(EVIDENCE_PIN_FILE, ordinary_in_pin_file);
    assert_eq!(found.len(), 1, "{found:?}");

    // A pinned test compares the boards as they are: it may not tamper.
    let tampering = r#"
        #[test]
        fn outside_edited_columns() {
            try_run_golden(&steps, |_, rust, _| { tamper(rust) }).unwrap_err();
        }
    "#;
    let found = violations(PIN_TABLE, tampering);
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(found[0].contains("a tampering hook"), "{found:?}");
}

/// The harness is reached by its own name, called directly (#2270 round-3
/// review L4): an import renaming it, or a use that is not a direct call,
/// would let a call past the walk.
#[test]
fn the_hook_checker_rejects_renamed_or_indirect_harness_uses() {
    let imports = r#"
        use swarm_board_diff::scenario::{run_golden, step, try_run_golden};
        use super::scenario::try_run_golden;
    "#;
    assert_eq!(violations(ELSEWHERE, imports), Vec::<String>::new());

    for renamed in [
        "use swarm_board_diff::scenario::try_run_golden as t;\n\
         fn a_scenario() { t(&steps, |_, rust, _| { tamper(rust) }).unwrap(); }",
        "use scenario::{step, try_run_golden as run};",
        "use scenario::{step, try_run_golden as try_run_golden};",
    ] {
        let found = violations(ELSEWHERE, renamed);
        assert_eq!(found.len(), 1, "{renamed}: {found:?}");
        assert!(found[0].contains("renames"), "{renamed}: {found:?}");
    }

    for indirect in [
        "fn a_scenario() { let run = try_run_golden; run(&steps, |_, rust, _| { tamper(rust) }).unwrap(); }",
        "fn a_scenario() { (try_run_golden)(&steps, |_, rust, _| { tamper(rust) }).unwrap(); }",
        "fn a_scenario() { apply(try_run_golden); }",
    ] {
        let found = violations(ELSEWHERE, indirect);
        assert_eq!(found.len(), 1, "{indirect}: {found:?}");
        assert!(
            found[0].contains("not called directly"),
            "{indirect}: {found:?}"
        );
    }
}

/// Only the pin table's own tests may expect a difference (#2270 round-4
/// review L2): a `#[test]` function of `swarm_board_diff_loose.rs` named
/// by `PERMITTED_DIVERGENCES` and pinned there (not by `EXTERNAL_PINS`),
/// whose harness call is the receiver of `.unwrap_err()` or
/// `.expect_err(..)`; and only the harness file's own `run_golden` compares
/// without one.
#[test]
fn the_hook_checker_permits_only_the_pin_tables_own_tests() {
    for (path, bypass) in [
        // A divergence pinned outside the suite names no caller here.
        (
            PIN_TABLE,
            "#[test] fn real_to_text_digits() { try_run_golden(&steps, |_, _, _| {}).unwrap_err(); }",
        ),
        (
            PIN_TABLE,
            "#[test] fn outside_edited_contract() { try_run_golden(&steps, |_, _, _| {}).unwrap_err(); }",
        ),
        // Nor does the external test pinning it.
        (
            PIN_TABLE,
            "#[test] fn a_pause_start_that_is_not_a_float_diverges() { try_run_golden(&steps, |_, _, _| {}).unwrap_err(); }",
        ),
        // The reviewer's bypass: a helper, not a test, discarding the Err.
        (
            PIN_TABLE,
            "fn real_to_text_digits() { let _ = try_run_golden(&steps, |_, _, _| {}); }",
        ),
        (
            PIN_TABLE,
            "fn outside_edited_columns() { try_run_golden(&steps, |_, _, _| {}).unwrap_err(); }",
        ),
        // A pinned name outside the pin table's file.
        (
            ELSEWHERE,
            "#[test] fn outside_edited_columns() { try_run_golden(&steps, |_, _, _| {}).unwrap_err(); }",
        ),
        // A pinned test that does not check the Err.
        (
            PIN_TABLE,
            "#[test] fn outside_edited_columns() { let _ = try_run_golden(&steps, |_, _, _| {}); }",
        ),
        (
            PIN_TABLE,
            "#[test] fn outside_edited_columns() { let held = try_run_golden(&steps, |_, _, _| {}); held.unwrap_err(); }",
        ),
        (
            PIN_TABLE,
            "#[test] fn outside_edited_columns() { try_run_golden(&steps, |_, _, _| {}).ok(); }",
        ),
        // A function nested in a pinned test is not that test.
        (
            PIN_TABLE,
            "#[test] fn outside_edited_columns() { fn outside_edited_columns() { let _ = try_run_golden(&steps, |_, _, _| {}); } }",
        ),
        // A `run_golden` of another file is not the harness's.
        (
            ELSEWHERE,
            "fn run_golden(steps: &[Step]) { let _ = try_run_golden(steps, |_, _, _| {}); }",
        ),
    ] {
        let found = violations(path, bypass);
        assert_eq!(found.len(), 1, "{path}: {bypass}: {found:?}");
        assert!(
            found[0].contains("outside a pinned divergence test"),
            "{path}: {bypass}: {found:?}"
        );
    }
}

/// The pin tables are the files the pin table lists in `PIN_TABLE_FILES`
/// (PR #2321 final review), read from its `include_str!` calls: itself,
/// the #2275 sibling holding pins and the submission scenarios holding
/// `outside_edited_evidence`'s second pin, and not the siblings holding
/// only pins of `EXTERNAL_PINS`. The second pin is a test of that file.
#[test]
fn the_pin_tables_are_the_files_pin_table_files_lists() {
    let listed = pin_table_files();
    for path in [
        PIN_TABLE,
        "tests/integration/swarm_board_diff_loose_files.rs",
        EVIDENCE_PIN_FILE,
    ] {
        assert!(listed.iter().any(|file| file == path), "{path}: {listed:?}");
        assert!(Role::of(path, &listed) == Role::PinTable, "{path}");
    }
    for path in [
        "tests/integration/swarm_board_diff_loose_runs.rs",
        "tests/integration/swarm_board_diff_loose_completion.rs",
    ] {
        assert!(listed.iter().all(|file| file != path), "{path}: {listed:?}");
        assert!(Role::of(path, &listed) == Role::Other, "{path}");
    }
    let evidence = ("outside_edited_evidence", EVIDENCE_PIN_TEST);
    let seconds = second_pins(&pin_table());
    assert_eq!(seconds, [(evidence.0.to_owned(), evidence.1.to_owned())]);
    let source = std::fs::read_to_string(EVIDENCE_PIN_FILE).expect("the second pin's file");
    assert!(source.contains(&format!("#[test]\nfn {EVIDENCE_PIN_TEST}()")));
    assert!(pinned_tests().contains(&EVIDENCE_PIN_TEST.to_owned()));
}

/// #2283 review M2: the fixture writers and the fixture-taking replay
/// (`write_fixture`, `record_rust`, `try_run_golden_in`) are the golden
/// self-tests' alone: anywhere else they could write a fixture, or compare
/// with one of a test's own making and so bypass the pin table.
#[test]
fn the_hook_checker_confines_fixture_writers_to_the_self_tests() {
    for call in [
        "record_rust(&steps);",
        "golden.write_fixture(dir, \"s\", &steps);",
        "Golden::write_fixture(&golden, dir, \"s\", &steps);",
        "let _ = try_run_golden_in(dir, \"s\", &steps);",
        "try_run_golden_in(dir, \"s\", &steps).unwrap_err();",
    ] {
        let ordinary = format!("#[test]\nfn a_scenario() {{ {call} }}");
        let found = violations(ELSEWHERE, &ordinary);
        assert_eq!(found.len(), 1, "{call}: {found:?}");
        assert!(
            found[0].contains("a golden fixture writer"),
            "{call}: {found:?}"
        );
        // Not in a pin either.
        let pinned = ordinary.replace("a_scenario", "outside_edited_columns");
        let found = violations(PIN_TABLE, &pinned);
        assert_eq!(found.len(), 1, "{call}: {found:?}");
        // A harness self-test, or the golden self-test in its file, may.
        let self_test = ordinary.replace("a_scenario", "harness_self_test_writes");
        assert_eq!(violations(ELSEWHERE, &self_test), Vec::<String>::new());
        let golden = ordinary.replace("a_scenario", GOLDEN_SELF_TEST.1);
        assert_eq!(
            violations(GOLDEN_SELF_TEST.0, &golden),
            Vec::<String>::new()
        );
        assert_eq!(violations(ELSEWHERE, &golden).len(), 1, "{call}");
    }
    // A writer used as a value, or renamed on import, hides its calls.
    for hidden in [
        "fn a_scenario() { apply(record_rust); }",
        "use scenario::record_rust as r;",
    ] {
        assert_eq!(violations(ELSEWHERE, hidden).len(), 1, "{hidden}");
    }
}
