use super::*;
use std::os::unix::process::ExitStatusExt;
use std::process::{Command, Stdio};

/// The environment variable that turns [`child_entry`] into a scenario.
const CHILD: &str = "QUECTO_2192_PANIC_HOOK_CHILD";

/// Where a child arms its crash record (the session `cli:child`).
const CHILD_BASE: &str = "QUECTO_2192_PANIC_HOOK_BASE";

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
    run_child_in(scenario, &std::env::temp_dir())
}

/// As [`run_child`], its crash record armed under `base`.
fn run_child_in(scenario: &str, base: &std::path::Path) -> std::process::Output {
    test_support::without_core_dumps(&mut Command::new(std::env::current_exe().unwrap()))
        .env(CHILD_BASE, base)
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

/// A value whose drop ends the process: a death during an unwind that no
/// hook sees (as a stack overflow or an out-of-memory abort would be).
struct ExitsOnDrop;
impl Drop for ExitsOnDrop {
    fn drop(&mut self) {
        // SAFETY: `_exit` runs no destructor or handler and never returns.
        unsafe { libc::_exit(test_support::FATAL_EXIT_CODE) }
    }
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

fn base_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var_os(CHILD_BASE).unwrap())
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
    let base = base_dir();
    // The turn scenarios keep an event log, as an agent with one does.
    let event_log = match scenario.starts_with("turn-") {
        true => {
            crate::infrastructure::persistence::audit_log::AuditLog::open_sync(&base, "cli:child")
                .unwrap()
                .crash_line()
        }
        false => None,
    };
    crash_record::prepare(crash_record::CrashTarget {
        base_dir: Some(base),
        session_key: Some("cli:child".into()),
        event_log,
    });
    crash_record::arm_prepared();
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
                let caught = tool_panic_scope::catch_in_call(|| -> u8 { panic!("handled") });
                let dir = crash_record::RecordDir::existing(&base_dir()).unwrap();
                let left = crash_record::SessionRecords::new("cli:child")
                    .read(&dir, Some(std::process::id()));
                println!("LEFT WHILE RUNNING {}", left.is_some());
                caught.is_err()
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
        // L-3: a provisional record the call never noted (the hook wrote it
        // as the call ended on another thread) is still withdrawn at its end.
        "unnoted" => {
            let scope = ToolScope::new("edit");
            let record = CrashRecord::new(PanicReport::new("unnoted", None), 1, 1).provisional();
            let under = crash_record::record_provisional(scope.id(), &record);
            println!("WRITTEN {under:?}");
            runtime.block_on(tool_panic_scope::scoped(scope, async {}));
        }
        "ends-while-unwinding" => contain(&runtime, "edit", async {
            tokio::task::yield_now().await;
            let _ends_the_process = ExitsOnDrop;
            panic!("first: the process ends before this is contained");
        }),
        "cancelled" => runtime.block_on(async {
            let (tx, rx) = std::sync::mpsc::channel::<()>();
            let call = tool_panic_scope::scoped(ToolScope::new("grep"), async move {
                // Carried work panics; the call is cancelled before it joins.
                let _join =
                    crate::infrastructure::tools::call_work::spawn_blocking_in_call(move || {
                        let _ = tx.send(());
                        panic!("carried work of a call that was then cancelled");
                    });
                std::future::pending::<()>().await;
            });
            let mut call = Box::pin(call);
            let _ = futures::poll!(call.as_mut());
            rx.recv().unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let dir = crash_record::RecordDir::existing(&base_dir()).unwrap();
            let left =
                crash_record::SessionRecords::new("cli:child").read(&dir, Some(std::process::id()));
            println!("WHILE RUNNING {}", left.is_some());
            drop(call);
        }),
        // #2192 review (F8): the fatal `error` event is filed under the
        // turn of the call it struck, or — outside any call — the turn of
        // the call started last.
        "turn-in-call" => runtime.block_on(tool_panic_scope::scoped(
            ToolScope::in_turn("edit", 7),
            async {
                tokio::task::yield_now().await;
                let _fails_while_unwinding = PanicsOnDrop;
                panic!("first: the tool's own panic in turn 7");
            },
        )),
        "turn-after-call" => {
            runtime.block_on(tool_panic_scope::scoped(
                ToolScope::in_turn("edit", 5),
                async {},
            ));
            panic!("outside any call, after turn 5's");
        }
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
            "quecto: fatal panic (tool calls running: edit; it struck the call's own panic \
             while that unwound: first: the tool's own panic at "
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
    let base = tempfile::tempdir().unwrap();
    let output = run_child_in("caught-only", base.path());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{:?} {stdout}", output.status);
    assert!(stdout.contains("ANSWER true RECORDED None"), "{stdout}");
    assert!(
        stdout.contains("LEFT WHILE RUNNING false"),
        "the handled panic's provisional record is withdrawn at the catch: {stdout}"
    );
    assert_eq!(child_record(base.path()), None);
}

#[test]
fn a_call_end_withdraws_a_provisional_record_it_never_noted() {
    let base = tempfile::tempdir().unwrap();
    let output = run_child_in("unnoted", base.path());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{output:?}");
    assert!(stdout.contains("WRITTEN Some(\"cli:child\")"), "{stdout}");
    assert_eq!(
        child_record(base.path()),
        None,
        "withdrawn at the call's end"
    );
}

/// A closed scope: a call that already ended.
/// A scope whose panic `message` was recorded while its call ran, and whose
/// call has ended since: the hook's record landed before the close, and
/// its provisional record is left after it.
fn ended_scope_with(tool: &str, message: &str) -> std::sync::Arc<ToolScope> {
    let scope = ToolScope::new(tool);
    let call = tool_panic_scope::scoped(scope.clone(), async {});
    assert_eq!(scope.record_panic(site(message)), Recording::Kept);
    drop(call);
    assert!(!scope.is_open());
    // A record after the close is refused: the panic is fatal then.
    assert_eq!(scope.record_panic(site("after")), Recording::Refused);
    scope
}

fn site(message: &str) -> PanicSite {
    PanicSite {
        message: message.into(),
        location: None,
    }
}

#[test]
fn a_provisional_record_left_while_its_call_ends_is_withdrawn_by_the_hook() {
    let scope = ended_scope_with("edit", "late");
    let mut withdrawn = Vec::new();
    leave_provisional(
        &scope,
        || Some("cli:a".to_string()),
        |id, under| withdrawn.push((id, under.map(str::to_string))),
    );
    assert_eq!(withdrawn, [(scope.id(), Some("cli:a".to_string()))]);
    assert_eq!(scope.take_provisional(), None, "withdrawn once");
}

#[test]
fn a_provisional_record_of_a_running_call_is_noted_for_its_end() {
    let scope = ToolScope::new("edit");
    assert_eq!(scope.record_panic(site("running")), Recording::Kept);
    let mut withdrawn = Vec::new();
    leave_provisional(
        &scope,
        || Some("cli:a".to_string()),
        |id, under| withdrawn.push((id, under.map(str::to_string))),
    );
    assert!(withdrawn.is_empty(), "the call's end withdraws it");
    assert_eq!(scope.take_provisional().as_deref(), Some("cli:a"));
    // Nothing written (disarmed): nothing noted, nothing withdrawn.
    let scope = ended_scope_with("edit", "unwritten");
    leave_provisional(
        &scope,
        || None,
        |id, under| withdrawn.push((id, under.map(str::to_string))),
    );
    assert!(withdrawn.is_empty());
}

/// The crash record a child left under `base`: any process's, since the
/// test does not learn the child's pid.
fn child_record(base: &std::path::Path) -> Option<CrashRecord> {
    crash_record::SessionRecords::new("cli:child")
        .read(&crash_record::RecordDir::existing(base).ok()?, None)
}

#[test]
fn a_fatal_panic_leaves_a_crash_record_after_reporting_it() {
    let base = tempfile::tempdir().unwrap();
    let output = run_child_in("outside", base.path());
    aborted(&output, "an ordinary bug outside any tool");
    let record = child_record(base.path()).expect("a crash record");
    assert_eq!(record.panic.message, "an ordinary bug outside any tool");
    assert!(
        record
            .panic
            .location
            .unwrap()
            .contains("panic_hook_tests.rs:")
    );
    assert_eq!(
        (record.call, record.earlier, record.provisional),
        (None, None, false)
    );
}

#[test]
fn a_contained_panic_leaves_no_record() {
    let base = tempfile::tempdir().unwrap();
    let output = run_child_in("tool", base.path());
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        child_record(base.path()),
        None,
        "the provisional record was withdrawn"
    );
}

#[test]
fn a_second_panic_during_the_unwind_is_recorded_with_both_panics() {
    let base = tempfile::tempdir().unwrap();
    let output = run_child_in("second", base.path());
    aborted(&output, "second: a destructor panicked during the unwind");
    let record = child_record(base.path()).expect("a crash record");
    assert_eq!(
        record.panic.message,
        "second: a destructor panicked during the unwind"
    );
    assert_eq!(record.call.as_deref(), Some("edit"));
    assert_eq!(
        record.earlier.map(|earlier| earlier.message).as_deref(),
        Some("first: the tool's own panic")
    );
}

#[test]
fn a_process_that_ends_before_its_call_contains_a_panic_leaves_the_provisional_record() {
    let base = tempfile::tempdir().unwrap();
    let output = run_child_in("ends-while-unwinding", base.path());
    assert_eq!(
        output.status.code(),
        Some(test_support::FATAL_EXIT_CODE),
        "{output:?}"
    );
    let record = child_record(base.path()).expect("the provisional record stayed");
    assert!(record.provisional);
    assert_eq!(record.call.as_deref(), Some("edit"));
    assert_eq!(
        record.panic.message,
        "first: the process ends before this is contained"
    );
}

#[test]
fn a_cancelled_call_withdraws_the_record_its_contained_panic_left() {
    let base = tempfile::tempdir().unwrap();
    let output = run_child_in("cancelled", base.path());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "{output:?}");
    assert!(stdout.contains("WHILE RUNNING true"), "{stdout}");
    assert_eq!(
        child_record(base.path()),
        None,
        "withdrawn when the call was dropped"
    );
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

/// The fatal `error` event a child left in its event log under `base`.
fn fatal_event(base: &std::path::Path) -> serde_json::Value {
    let path =
        crate::infrastructure::persistence::audit_log::AuditLog::file_path(base, "cli:child");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    text.lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|event| event["event"] == "error" && event["source"] == "panic")
        .unwrap_or_else(|| panic!("no fatal error event in {text:?}"))
}

/// #2192 review (F8): the real hook files the fatal event under the turn
/// it happened in — the struck call's, or the last call's outside one.
#[test]
fn the_fatal_event_is_filed_under_the_turn_it_happened_in() {
    let base = tempfile::tempdir().unwrap();
    let output = run_child_in("turn-in-call", base.path());
    aborted(&output, "second: a destructor panicked during the unwind");
    let event = fatal_event(base.path());
    assert_eq!(event["turn"], 7, "{event}");
    assert_eq!(event["tool"], "edit", "{event}");

    let base = tempfile::tempdir().unwrap();
    let output = run_child_in("turn-after-call", base.path());
    aborted(&output, "outside any call, after turn 5's");
    let event = fatal_event(base.path());
    assert_eq!(event["turn"], 5, "{event}");
    assert!(event["tool"].is_null(), "{event}");
}
