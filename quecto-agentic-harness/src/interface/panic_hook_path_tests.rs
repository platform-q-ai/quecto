//! What runs inside the panic hook (#2192 review): nothing it calls prints
//! through a macro that panics on a failed write, or panics itself; and the
//! location it records is relative to its crate.
use super::*;

/// #2192 review (PRRT_kwDORUxnPM6mf9Bd): a panic's location never records
/// an absolute build path.
#[test]
fn a_panic_location_is_recorded_relative_to_its_crate() {
    assert_eq!(
        workspace_location("/home/u/work/quecto/quecto-agentic-harness/src/tools/edit.rs"),
        "quecto-agentic-harness/src/tools/edit.rs"
    );
    assert_eq!(
        workspace_location(
            "/home/u/.cargo/registry/src/index.crates.io-1/tokio-1.47.0/src/runtime/task.rs"
        ),
        "tokio-1.47.0/src/runtime/task.rs",
        "the last /src/ decides"
    );
    assert_eq!(
        workspace_location("quecto-agentic-harness/src/a.rs"),
        "quecto-agentic-harness/src/a.rs",
        "a relative path is kept"
    );
    assert_eq!(workspace_location("/build/generated.rs"), "generated.rs");
    assert_eq!(workspace_location("/src/a.rs"), "src/a.rs");
}

/// The functions the hook calls, by file: what runs inside the panic hook.
const HOOK_PATH: &[(&str, &[&str])] = &[
    (
        include_str!("panic_hook.rs"),
        &[
            "disposition",
            "can_unwind",
            "can_unwind_in",
            "contained_report",
            "report_line",
            "fatal_context",
            "panic_site",
            "workspace_location",
            "crash",
            "record_fatal",
            "leave_provisional",
        ],
    ),
    (
        include_str!("../application/tool_panic_scope.rs"),
        &[
            "current",
            "begin_contained_unwind",
            "in_flight_tools",
            "payload_message",
            "thread_token",
            "record_panic",
            "recorded_panic",
            "with_site",
            "record",
            "has_thread",
            "is_open",
            "tool",
            "note_provisional",
            "take_provisional",
            "id",
            "turn",
        ],
    ),
];

/// Macros and methods that print (and panic on a failed write) or panic:
/// in the hook, either is an abort without the report.
const PANICKING_CALLS: &[&str] = &[
    "eprintln",
    "eprint",
    "println",
    "print",
    "panic",
    "assert",
    "assert_eq",
    "assert_ne",
    "unreachable",
    "todo",
    "unimplemented",
    "unwrap",
    "expect",
];

/// The macro and method names each function of `source` calls, by name.
fn calls_by_function(source: &str) -> std::collections::HashMap<String, Vec<String>> {
    use syn::visit::Visit;
    #[derive(Default)]
    struct Calls(Vec<String>);
    impl<'a> Visit<'a> for Calls {
        fn visit_macro(&mut self, mac: &'a syn::Macro) {
            if let Some(last) = mac.path.segments.last() {
                self.0.push(last.ident.to_string());
            }
        }
        fn visit_expr_method_call(&mut self, call: &'a syn::ExprMethodCall) {
            self.0.push(call.method.to_string());
            syn::visit::visit_expr_method_call(self, call);
        }
    }
    let file = syn::parse_file(source).expect("the source parses");
    let mut found = std::collections::HashMap::new();
    let mut add = |name: String, block: &syn::Block| {
        let mut calls = Calls::default();
        calls.visit_block(block);
        found.insert(name, calls.0);
    };
    for item in &file.items {
        match item {
            syn::Item::Fn(function) => add(function.sig.ident.to_string(), &function.block),
            syn::Item::Impl(implementation) => {
                for inner in &implementation.items {
                    if let syn::ImplItem::Fn(method) = inner {
                        add(method.sig.ident.to_string(), &method.block);
                    }
                }
            }
            _ => {}
        }
    }
    found
}

/// #2192 review (PRRT_kwDORUxnPM6mf9Bk): nothing the hook calls prints
/// through a macro that panics on a failed write, or panics itself.
#[test]
fn nothing_the_hook_calls_prints_or_panics() {
    for (source, functions) in HOOK_PATH {
        let calls = calls_by_function(source);
        for function in *functions {
            let called = calls
                .get(*function)
                .unwrap_or_else(|| panic!("the hook-path function {function} exists"));
            let banned: Vec<&String> = called
                .iter()
                .filter(|name| PANICKING_CALLS.contains(&name.as_str()))
                .collect();
            assert!(banned.is_empty(), "{function} calls {banned:?} in the hook");
        }
    }
}

/// #2192 review (PRRT_kwDORUxnPM6mf9Bb): a crash record — and so the fatal
/// `error` event — carries a panic's message with secret shapes redacted,
/// and bounded.
#[test]
fn a_crash_record_redacts_and_bounds_the_panic_message() {
    let secret = PanicSite {
        message: "request failed with api_key=sk-abcdefghijklmnopqrstuv".into(),
        location: Some("quecto-agentic-harness/src/a.rs:1:2".into()),
    };
    let record = crash(&secret, Some(&secret));
    for text in [
        &record.panic.message,
        &record.earlier.as_ref().unwrap().message,
    ] {
        assert!(!text.contains("sk-abcdefghijklmnopqrstuv"), "{text}");
        assert!(text.contains("[REDACTED]"), "{text}");
    }
    let huge = PanicSite {
        message: "x".repeat(100_000),
        location: None,
    };
    assert!(
        crash(&huge, None).panic.message.len() <= crate::domain::crash_record::MAX_CRASH_TEXT_BYTES
    );
}
