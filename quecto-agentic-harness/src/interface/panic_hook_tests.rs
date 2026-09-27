use super::*;
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};

/// The environment variable that turns [`child_entry`] into a scenario.
const CHILD: &str = "QUECTO_2192_PANIC_HOOK_CHILD";

/// Compared by address, as `quecto_fail_fast`'s own
/// `the_production_end_is_an_abort` explains: an abort cannot be run by a
/// test, and no type tells one diverging function from another.
#[test]
fn a_production_build_ends_a_fatal_panic_with_an_abort() {
    let abort = std::process::abort as fn() -> ! as usize;
    assert_eq!(FATAL_END as usize, abort);
    assert_eq!(
        FATAL_END as usize,
        quecto_fail_fast::FATAL_END as usize,
        "one end"
    );
    // Only the switch's value `1` selects another end; this reads no
    // environment, so it holds whatever this test process's own is.
    for switch in [None, Some("0"), Some(""), Some("yes")] {
        let end = test_support::fatal_end_for(switch, FATAL_END);
        assert_eq!(end as usize, abort, "{switch:?}");
    }
    let test_end = test_support::fatal_end_for(Some("1"), FATAL_END);
    assert_ne!(test_end as usize, abort);
}

#[test]
fn a_fatal_panic_names_the_calls_running_and_the_panic_it_struck() {
    assert_eq!(fatal_context(&[], None), None);
    assert_eq!(
        fatal_context(&["edit".into(), "grep".into()], None).as_deref(),
        Some("quecto: fatal panic (tool calls running: edit, grep)")
    );
    let struck = PanicSite {
        message: "first".into(),
        location: Some("a.rs:1:2".into()),
    };
    assert_eq!(
        fatal_context(&["edit".into()], Some(&struck)).as_deref(),
        Some(
            "quecto: fatal panic (tool calls running: edit; it struck the call's own panic \
             while that unwound: first at a.rs:1:2)"
        )
    );
    let unlocated = PanicSite {
        message: "first".into(),
        location: None,
    };
    assert_eq!(
        fatal_context(&[], Some(&unlocated)).as_deref(),
        Some(
            "quecto: fatal panic (it struck the call's own panic while that unwound: first at \
             an unknown location)"
        )
    );
}

#[test]
fn a_panic_outside_a_tool_scope_aborts() {
    assert_eq!(disposition(None, false, true), Disposition::Abort);
    assert_eq!(disposition(None, true, true), Disposition::Abort);
}

#[test]
fn a_first_panic_inside_a_tool_scope_is_contained_and_a_second_is_not() {
    let scope = ToolScope::new("edit");
    assert_eq!(disposition(Some(&scope), false, true), Disposition::Contain);
    assert_eq!(disposition(Some(&scope), true, true), Disposition::Abort);
}

#[test]
fn a_panic_that_cannot_unwind_is_fatal_even_inside_a_tool_scope() {
    let scope = ToolScope::new("edit");
    assert_eq!(disposition(Some(&scope), false, false), Disposition::Abort);
    assert_eq!(disposition(None, false, false), Disposition::Abort);
}

#[test]
fn whether_a_panic_can_unwind_is_read_from_the_hook_info_debug_text() {
    let unwinding = "PanicHookInfo { payload: Any { .. }, location: Location { file: \"a.rs\", \
                     line: 1, column: 2 }, can_unwind: true, force_no_backtrace: false }";
    assert!(can_unwind_in(unwinding));
    assert!(!can_unwind_in(
        &unwinding.replace("can_unwind: true", "can_unwind: false")
    ));
    // A file named like the field does not decide it: the field comes last.
    let tricky = unwinding.replace("a.rs", "can_unwind: false.rs");
    assert!(can_unwind_in(&tricky));
    let tricky = unwinding
        .replace("a.rs", "can_unwind: true.rs")
        .replace("can_unwind: true,", "can_unwind: false,");
    assert!(!can_unwind_in(&tricky));
    // #2192 review L2: only an explicit `can_unwind: true` says it can; a
    // text that does not say, or says something else, is taken as fatal.
    assert!(!can_unwind_in("PanicHookInfo { .. }"));
    assert!(!can_unwind_in(""));
    assert!(!can_unwind_in("can_unwind: maybe"));
    assert!(!can_unwind_in(
        "can_unwind: trueish, force_no_backtrace: false }"
    ));
    assert!(!can_unwind_in(
        &unwinding.replace("can_unwind: true", "can_unwind:true")
    ));
    assert!(can_unwind_in("can_unwind: true }"));
    assert!(can_unwind_in("can_unwind: true"));
}

#[test]
fn a_panic_whose_unwinding_is_unknown_is_fatal_inside_a_tool_scope() {
    let scope = ToolScope::new("edit");
    let unknown = can_unwind_in("PanicHookInfo { payload: Any { .. } }");
    assert_eq!(
        disposition(Some(&scope), false, unknown),
        Disposition::Abort
    );
}

#[test]
fn the_hook_prints_through_a_write_that_cannot_panic() {
    let source = include_str!("panic_hook.rs");
    let production = source.split("#[cfg(test)]").next().unwrap();
    for macro_name in ["eprintln!", "eprint!", "println!", "print!"] {
        assert!(
            !production.contains(macro_name),
            "{macro_name} panics on a failed write, which in the hook is an abort"
        );
    }
}

#[test]
fn the_contained_report_names_the_tool_where_and_why() {
    let site = PanicSite {
        message: "boom".into(),
        location: Some("a.rs:1:2".into()),
    };
    assert_eq!(
        contained_report("edit", &site),
        "quecto: tool 'edit' panicked at a.rs:1:2: boom (contained to its call unless a fatal report follows)"
    );
    let unlocated = PanicSite {
        message: "boom".into(),
        location: None,
    };
    assert_eq!(
        contained_report("edit", &unlocated),
        "quecto: tool 'edit' panicked at an unknown location: boom (contained to its call unless a fatal report follows)"
    );
}

/// Run this test binary again with only [`child_entry`], as `scenario`.
fn run_child(scenario: &str) -> std::process::Output {
    test_support::without_core_dumps(&mut Command::new(std::env::current_exe().unwrap()))
        .args([
            "--exact",
            "interface::panic_hook::tests::child_entry",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD, scenario)
        .env("RUST_BACKTRACE", "0")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

/// A value whose drop panics: a destructor that fails during an unwind.
struct PanicsOnDrop;
impl Drop for PanicsOnDrop {
    fn drop(&mut self) {
        panic!("second: a destructor panicked during the unwind");
    }
}

fn contain<F: std::future::Future>(runtime: &tokio::runtime::Runtime, tool: &str, call: F) {
    use futures::FutureExt;
    let scope = ToolScope::new(tool);
    let call = tool_panic_scope::scoped(scope.clone(), call);
    let outcome = runtime.block_on(std::panic::AssertUnwindSafe(call).catch_unwind());
    assert!(outcome.is_err(), "the call panicked");
    let site = scope.recorded_panic().expect("the hook recorded the panic");
    println!(
        "CONTAINED {} @ {}",
        site.message,
        site.location.unwrap_or_default()
    );
}

/// The child side of the process tests: a no-op in an ordinary run.
#[test]
fn child_entry() {
    let Ok(scenario) = std::env::var(CHILD) else {
        return;
    };
    if scenario == "debug-format" {
        // The real text `can_unwind_in` reads, from a real hook's view.
        std::panic::set_hook(Box::new(|info| println!("DEBUG {info:?}")));
        let _ = std::panic::catch_unwind(|| panic!("seen"));
        return;
    }
    install();
    install(); // a second install is a no-op, not a second hook
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    match scenario.as_str() {
        "outside" => panic!("an ordinary bug outside any tool"),
        "tool" => contain(&runtime, "edit", async {
            tokio::task::yield_now().await;
            let text = "’’ab’";
            let index = text.len() - 2;
            text[..index].len()
        }),
        "carried" => contain(&runtime, "grep", async {
            let joined = crate::infrastructure::tools::call_work::spawn_blocking_in_call(|| {
                panic!("carried blocking work")
            })
            .await;
            println!("UNREACHABLE {}", joined.is_ok());
        }),
        "second" => contain(&runtime, "edit", async {
            tokio::task::yield_now().await;
            let _fails_while_unwinding = PanicsOnDrop;
            panic!("first: the tool's own panic");
        }),
        "uncarried" => runtime.block_on(tool_panic_scope::scoped(ToolScope::new("grep"), async {
            let _ = tokio::task::spawn_blocking(|| panic!("uncarried background work")).await;
            println!("UNREACHABLE");
        })),
        "leftover" => runtime.block_on(async {
            let (tx, rx) = tokio::sync::oneshot::channel::<()>();
            tool_panic_scope::scoped(ToolScope::new("bash"), async {
                // A reader the call leaves running when it returns.
                drop(crate::infrastructure::tools::call_work::spawn_in_call(
                    async {
                        let _ = rx.await;
                        panic!("the call's leftover reader");
                    },
                ));
            })
            .await;
            tx.send(()).unwrap();
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            println!("UNREACHABLE");
        }),
        // A tool that handles its own panic (#2192 F1), then really panics:
        // the real one is contained, not taken for a second panic.
        "caught-then-real" => contain(&runtime, "edit", async {
            tokio::task::yield_now().await;
            let handled = tool_panic_scope::catch_in_call(|| -> u8 { panic!("handled") });
            println!("HANDLED {}", handled.is_err());
            panic!("the real one");
        }),
        "caught-only" => {
            let scope = ToolScope::new("edit");
            let answer = runtime.block_on(tool_panic_scope::scoped(scope.clone(), async {
                tool_panic_scope::catch_in_call(|| -> u8 { panic!("handled") }).is_err()
            }));
            println!("ANSWER {answer} RECORDED {:?}", scope.recorded_panic());
        }
        // #2192 review L1: a panic inside an `extern "C"` function first
        // unwinds like any other (the hook contains it), then Rust stops it
        // at the function's no-unwind boundary with a second panic that
        // cannot unwind — fatal, and reported with the first.
        "extern-c" => contain(&runtime, "edit", async {
            extern "C" fn cannot_unwind_out() {
                panic!("inside an extern C function");
            }
            tokio::task::yield_now().await;
            cannot_unwind_out();
            println!("UNREACHABLE");
        }),
        "core-limit" => {
            let mut limit = libc::rlimit {
                rlim_cur: 1,
                rlim_max: 1,
            };
            // SAFETY: getrlimit writes only into the struct it is given.
            let status = unsafe { libc::getrlimit(libc::RLIMIT_CORE, &mut limit) };
            assert_eq!(status, 0);
            println!("CORE LIMIT {} {}", limit.rlim_cur, limit.rlim_max);
        }
        other => panic!("unknown scenario {other}"),
    }
}

/// The child ended fatally — with the test-support `_exit(134)` that
/// stands in for an abort, never a core-dumping signal — after reporting.
fn aborted(output: &std::process::Output, message: &str) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        (output.status.code(), output.status.signal()),
        (Some(test_support::FATAL_EXIT_CODE), None),
        "{:?} {stdout} {stderr}",
        output.status
    );
    assert!(stderr.contains(message), "reported on stderr: {stderr}");
    assert!(!stdout.contains("UNREACHABLE"), "{stdout}");
}

#[test]
fn a_process_aborts_on_a_panic_outside_a_tool() {
    aborted(&run_child("outside"), "an ordinary bug outside any tool");
}

#[test]
fn a_process_survives_a_panic_inside_a_tool_and_learns_where_it_was() {
    let output = run_child("tool");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{:?} {stdout}", output.status);
    assert!(
        stdout.contains("CONTAINED end byte index 9 is not a char boundary"),
        "{stdout}"
    );
    assert!(stdout.contains("panic_hook_tests.rs:"), "{stdout}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("quecto: tool 'edit' panicked at ")
            && stderr.contains("panic_hook_tests.rs:")
            && stderr.contains(": end byte index 9 is not a char boundary")
            && stderr.contains("(contained to its call unless a fatal report follows)")
            && stderr
                .lines()
                .filter(|line| line.starts_with("quecto: tool "))
                .count()
                == 1,
        "a contained panic leaves one line on stderr: {stderr}"
    );
}

#[test]
fn the_hook_reads_the_real_panic_info_text() {
    let output = run_child("debug-format");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout
        .lines()
        .find_map(|line| line.split_once("DEBUG ").map(|(_, info)| info))
        .unwrap_or_else(|| panic!("the child printed its hook info: {stdout}"));
    assert!(line.contains("can_unwind: true"), "{line}");
    assert!(can_unwind_in(line), "{line}");
    assert!(!can_unwind_in(
        &line.replace("can_unwind: true", "can_unwind: false")
    ));
}

#[test]
fn a_panic_in_carried_work_resumes_in_its_call_with_its_own_location() {
    let output = run_child("carried");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{:?} {stdout}", output.status);
    assert!(
        stdout.contains("CONTAINED carried blocking work @ "),
        "{stdout}"
    );
    assert!(!stdout.contains("UNREACHABLE"), "{stdout}");
}

#[test]
fn a_second_panic_while_a_contained_one_unwinds_aborts_and_is_reported() {
    let output = run_child("second");
    aborted(&output, "second: a destructor panicked during the unwind");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(
            "quecto: fatal panic (tool calls running: edit; it struck the call's own \
             panic while that unwound: first: the tool's own panic at "
        ),
        "{stderr}"
    );
}

#[test]
fn uncarried_work_a_tool_hands_off_stays_fatal() {
    aborted(&run_child("uncarried"), "uncarried background work");
}

#[test]
fn work_left_running_after_its_call_ended_is_outside_it_and_fatal() {
    aborted(&run_child("leftover"), "the call's leftover reader");
}

#[test]
fn a_child_expected_to_end_fatally_has_no_core_limit_either() {
    let output = run_child("core-limit");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{stdout}");
    assert!(stdout.contains("CORE LIMIT 0 0"), "{stdout}");
}

#[test]
fn a_panic_the_tool_handled_is_not_taken_for_a_second_one() {
    let output = run_child("caught-then-real");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "{:?} {stdout} {stderr}",
        output.status
    );
    assert!(stdout.contains("HANDLED true"), "{stdout}");
    assert!(stdout.contains("CONTAINED the real one @ "), "{stdout}");
}

#[test]
fn a_panic_the_tool_handled_leaves_its_call_clean() {
    let output = run_child("caught-only");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{:?} {stdout}", output.status);
    assert!(stdout.contains("ANSWER true RECORDED None"), "{stdout}");
}

#[test]
fn a_panic_in_an_extern_c_function_is_fatal_at_its_boundary_and_names_the_first() {
    let output = run_child("extern-c");
    aborted(&output, "panic in a function that cannot unwind");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let contained = stderr
        .lines()
        .find(|line| line.starts_with("quecto: tool 'edit' panicked at "))
        .unwrap_or_else(|| panic!("the first panic was reported as contained: {stderr}"));
    assert!(
        contained.contains("inside an extern C function")
            && contained.ends_with("(contained to its call unless a fatal report follows)"),
        "the contained line says it may still become fatal: {contained}"
    );
    assert!(
        stderr.contains(
            "quecto: fatal panic (tool calls running: edit; it struck the call's own \
             panic while that unwound: inside an extern C function at "
        ),
        "{stderr}"
    );
}

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
