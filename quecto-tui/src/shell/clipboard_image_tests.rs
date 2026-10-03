//! #2425: the clipboard adapter against fake `wl-paste` / `xclip`
//! executables in a temporary directory; no test touches the real clipboard.

use super::*;
use crate::shell::test_executable::write_executable;
use std::path::Path;

const QUICK: Duration = Duration::from_secs(5);

/// A fake tool at `dir/name`: `cases` maps its exact argument string to
/// the shell that answers it; any other call exits 64.
fn fake_tool(dir: &Path, name: &str, cases: &[(&str, &str)]) -> PathBuf {
    let mut script = String::from("#!/bin/sh\ncase \"$*\" in\n");
    for (args, answer) in cases {
        script.push_str(&format!("  \"{args}\") {answer} ;;\n"));
    }
    script.push_str("  *) echo \"unexpected: $*\" >&2; exit 64 ;;\nesac\n");
    let path = dir.join(name);
    write_executable(&path, script);
    path
}

fn png_file(dir: &Path) -> (PathBuf, Vec<u8>) {
    let bytes = quecto_image::samples::png(3, 2);
    let path = dir.join("clip.png");
    std::fs::write(&path, &bytes).unwrap();
    (path, bytes)
}

fn wl_only(path: PathBuf) -> SystemClipboard {
    SystemClipboard::with_tools(vec![(ClipboardTool::WlPaste, path)], QUICK)
}

#[test]
fn wl_paste_offering_a_png_gives_its_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let (png, bytes) = png_file(dir.path());
    let cat = format!("exec cat '{}'", png.display());
    let tool = fake_tool(
        dir.path(),
        "wl-paste",
        &[
            (
                "--list-types",
                "printf 'text/html\\nimage/png\\ntext/plain\\n'",
            ),
            ("--type image/png", &cat),
        ],
    );
    assert_eq!(wl_only(tool).read(), ClipboardRead::Image(bytes));
}

#[test]
fn the_first_admitted_type_in_quecto_s_order_is_read() {
    let dir = tempfile::tempdir().unwrap();
    let (png, bytes) = png_file(dir.path());
    let cat = format!("exec cat '{}'", png.display());
    let tool = fake_tool(
        dir.path(),
        "wl-paste",
        &[
            ("--list-types", "printf 'image/webp\\nimage/png\\n'"),
            ("--type image/png", &cat),
        ],
    );
    assert_eq!(wl_only(tool).read(), ClipboardRead::Image(bytes));
}

#[test]
fn xclip_reads_targets_then_the_image() {
    let dir = tempfile::tempdir().unwrap();
    let (png, bytes) = png_file(dir.path());
    let cat = format!("exec cat '{}'", png.display());
    let tool = fake_tool(
        dir.path(),
        "xclip",
        &[
            (
                "-selection clipboard -t TARGETS -o",
                "printf 'TARGETS\\nimage/png\\n'",
            ),
            ("-selection clipboard -t image/png -o", &cat),
        ],
    );
    let clipboard = SystemClipboard::with_tools(vec![(ClipboardTool::Xclip, tool)], QUICK);
    assert_eq!(clipboard.read(), ClipboardRead::Image(bytes));
}

#[test]
fn text_without_an_image_is_read_as_text() {
    let dir = tempfile::tempdir().unwrap();
    let tool = fake_tool(
        dir.path(),
        "wl-paste",
        &[
            (
                "--list-types",
                "printf 'text/plain;charset=utf-8\\nUTF8_STRING\\n'",
            ),
            ("--no-newline", "printf 'hello world'"),
        ],
    );
    assert_eq!(
        wl_only(tool).read(),
        ClipboardRead::Text("hello world".into())
    );
}

#[test]
fn xclip_text_is_read_from_the_clipboard_selection() {
    let dir = tempfile::tempdir().unwrap();
    let tool = fake_tool(
        dir.path(),
        "xclip",
        &[
            (
                "-selection clipboard -t TARGETS -o",
                "printf 'TARGETS\\nUTF8_STRING\\nSTRING\\n'",
            ),
            ("-selection clipboard -o", "printf 'from x11'"),
        ],
    );
    let clipboard = SystemClipboard::with_tools(vec![(ClipboardTool::Xclip, tool)], QUICK);
    assert_eq!(clipboard.read(), ClipboardRead::Text("from x11".into()));
}

#[test]
fn an_image_of_a_type_quecto_does_not_admit_is_named() {
    let dir = tempfile::tempdir().unwrap();
    let tool = fake_tool(
        dir.path(),
        "wl-paste",
        &[("--list-types", "printf 'image/bmp\\nimage/tiff\\n'")],
    );
    assert_eq!(
        wl_only(tool).read(),
        ClipboardRead::UnsupportedImage("image/bmp, image/tiff".into())
    );
}

#[test]
fn an_empty_clipboard_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    let silent = fake_tool(dir.path(), "wl-paste", &[("--list-types", "true")]);
    assert_eq!(wl_only(silent).read(), ClipboardRead::Empty);

    // wl-paste exits 1 ("Nothing is copied") on an empty clipboard.
    let nothing = fake_tool(
        dir.path(),
        "wl-paste-2",
        &[("--list-types", "echo 'Nothing is copied' >&2; exit 1")],
    );
    assert_eq!(wl_only(nothing).read(), ClipboardRead::Empty);
}

#[test]
fn a_missing_tool_falls_through_to_the_next_and_none_means_no_tool() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("no-such-wl-paste");
    let xclip = fake_tool(
        dir.path(),
        "xclip",
        &[
            ("-selection clipboard -t TARGETS -o", "printf 'STRING\\n'"),
            ("-selection clipboard -o", "printf 'x'"),
        ],
    );
    let both = SystemClipboard::with_tools(
        vec![
            (ClipboardTool::WlPaste, missing.clone()),
            (ClipboardTool::Xclip, xclip),
        ],
        QUICK,
    );
    assert_eq!(both.read(), ClipboardRead::Text("x".into()));

    let neither = SystemClipboard::with_tools(vec![(ClipboardTool::WlPaste, missing)], QUICK);
    assert_eq!(neither.read(), ClipboardRead::NoTool);
    assert_eq!(
        SystemClipboard::with_tools(Vec::new(), QUICK).read(),
        ClipboardRead::NoTool
    );
}

#[test]
fn a_hung_tool_is_killed_at_the_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let tool = fake_tool(dir.path(), "wl-paste", &[("--list-types", "exec sleep 30")]);
    let clipboard = SystemClipboard::with_tools(
        vec![(ClipboardTool::WlPaste, tool)],
        Duration::from_millis(200),
    );
    let started = std::time::Instant::now();
    let read = clipboard.read();
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the read waited {:?}",
        started.elapsed()
    );
    match read {
        ClipboardRead::Failed(reason) => {
            assert!(reason.contains("timed out"), "{reason}");
            assert!(reason.contains("wl-paste"), "{reason}");
        }
        other => panic!("expected a timeout, got {other:?}"),
    }
}

#[test]
fn a_failing_image_read_is_a_failure() {
    let dir = tempfile::tempdir().unwrap();
    let tool = fake_tool(
        dir.path(),
        "wl-paste",
        &[
            ("--list-types", "printf 'image/png\\n'"),
            ("--type image/png", "echo 'broken pipe' >&2; exit 3"),
        ],
    );
    match wl_only(tool).read() {
        ClipboardRead::Failed(reason) => assert!(reason.contains("wl-paste"), "{reason}"),
        other => panic!("expected a failure, got {other:?}"),
    }
}

#[test]
fn an_image_read_stops_one_byte_past_the_limit() {
    let dir = tempfile::tempdir().unwrap();
    let tool = fake_tool(
        dir.path(),
        "wl-paste",
        &[
            ("--list-types", "printf 'image/png\\n'"),
            ("--type image/png", "exec head -c 6000000 /dev/zero"),
        ],
    );
    match wl_only(tool).read() {
        // The over-limit bytes go on to admission, which refuses them.
        ClipboardRead::Image(bytes) => {
            assert_eq!(bytes.len(), quecto_image::MAX_IMAGE_BYTES + 1)
        }
        other => panic!("expected the capped bytes, got {other:?}"),
    }
}

#[test]
fn a_session_uses_the_tools_its_environment_names() {
    use ClipboardTool::{WlPaste, Xclip};
    assert_eq!(ClipboardTool::for_session(true, true), [WlPaste, Xclip]);
    assert_eq!(ClipboardTool::for_session(true, false), [WlPaste]);
    assert_eq!(ClipboardTool::for_session(false, true), [Xclip]);
    assert!(ClipboardTool::for_session(false, false).is_empty());
}

#[test]
fn reads_never_print_image_bytes() {
    let debug = format!("{:?}", ClipboardRead::Image(vec![0x89; 4096]));
    assert_eq!(debug, "Image(4096 bytes)");
}

/// The state letter of `pid` in `/proc/<pid>/stat`, or `None` when it has
/// no entry (reaped, or reaped between the open and the read).
fn process_state(pid: libc::pid_t) -> Option<char> {
    assert!(pid > 0, "a process id is positive, got {pid}");
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => {
            // `pid (comm) S …`: the state follows the last `)`, as `comm`
            // may hold one.
            let state = stat
                .rsplit_once(')')
                .and_then(|(_, rest)| rest.trim_start().chars().next());
            Some(state.unwrap_or_else(|| panic!("/proc/{pid}/stat has a state: {stat:?}")))
        }
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound
                || error.raw_os_error() == Some(libc::ESRCH) =>
        {
            None
        }
        Err(error) => panic!("read /proc/{pid}/stat: {error}"),
    }
}

/// Whether a process in `state` is gone: no entry, or one that has exited
/// and only waits for its reaper (`Z`ombie, or `X`, dead and being
/// released). A SIGKILLed helper is gone once it is a zombie;
/// `kill(pid, 0)` still succeeds on one, so it cannot tell.
fn state_is_gone(state: Option<char>) -> bool {
    match state {
        None => true,
        Some(state) => matches!(state, 'Z' | 'X'),
    }
}

fn process_gone(pid: libc::pid_t) -> bool {
    state_is_gone(process_state(pid))
}

/// How long a killed helper is given to go (#2442): it is a survivor only
/// once both are spent. The polls are real 10 ms sleeps, so a stalled
/// runner that jumps past the wall-clock budget still gives the helper its
/// turn; the wall clock covers a helper starved or stuck in `D` state.
#[derive(Clone, Copy)]
struct GoneBudget {
    polls: u32,
    wall: Duration,
}

const HELPER_GONE_BUDGET: GoneBudget = GoneBudget {
    polls: 200,
    wall: Duration::from_secs(10),
};

/// `Ok` as soon as `pid` is gone; once `budget` is spent, `Err` naming its
/// last state and kernel wait channel.
fn wait_gone(pid: libc::pid_t, budget: GoneBudget) -> Result<(), String> {
    let started = std::time::Instant::now();
    let mut polls = 0;
    loop {
        let state = process_state(pid);
        if state_is_gone(state) {
            return Ok(());
        }
        if polls >= budget.polls && started.elapsed() >= budget.wall {
            let state = state.expect("a process that is not gone has a state");
            let wchan = std::fs::read_to_string(format!("/proc/{pid}/wchan"))
                .unwrap_or_else(|error| format!("unreadable: {error}"));
            return Err(format!(
                "state {state}, wchan {wchan:?} after {polls} polls and {:?} \
                 (S: never signalled, a product bug; R or D: the runner stalled)",
                started.elapsed()
            ));
        }
        polls += 1;
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The pid the tool's shell wrote to `pid_file`, waiting (bounded) for the
/// file to exist and hold a whole line.
fn helper_pid(pid_file: &Path) -> libc::pid_t {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    let written = loop {
        match std::fs::read_to_string(pid_file) {
            Ok(written) if written.ends_with('\n') => break written,
            read => assert!(
                std::time::Instant::now() < deadline,
                "the helper pid file {} is written: {read:?}",
                pid_file.display()
            ),
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let pid = written
        .trim()
        .parse()
        .unwrap_or_else(|error| panic!("the helper pid file holds a pid ({written:?}): {error}"));
    assert!(
        pid > 0,
        "the helper pid file holds a positive pid, got {pid}"
    );
    pid
}

/// Review round 1 (M2): `wl-paste` forks a helper; the timeout ends the
/// whole process group, so the helper does not outlive the read.
#[test]
fn a_timeout_ends_the_tool_s_children_too() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("helper.pid");
    let script = format!("sleep 30 & echo $! > '{}'; wait", pid_file.display());
    let tool = fake_tool(dir.path(), "wl-paste", &[("--list-types", &script)]);
    // 1 s, so the shell has long written `$!` when the timeout fires.
    let clipboard =
        SystemClipboard::with_tools(vec![(ClipboardTool::WlPaste, tool)], Duration::from_secs(1));
    let started = std::time::Instant::now();
    assert!(matches!(clipboard.read(), ClipboardRead::Failed(_)));
    // The tool's shell waits for its helper: a read that waited the
    // helper's 30 s out never killed it.
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "the read waited {:?}",
        started.elapsed()
    );
    let pid = helper_pid(&pid_file);
    if let Err(survivor) = wait_gone(pid, HELPER_GONE_BUDGET) {
        panic!("the forked helper {pid} outlived the read: {survivor}");
    }
}

/// #2442: a child that has exited but is not reaped is a zombie: gone,
/// though `kill(pid, 0)` still succeeds on it.
#[test]
fn a_zombie_counts_as_gone() {
    let mut child = std::process::Command::new("true").spawn().unwrap();
    let pid = libc::pid_t::try_from(child.id()).unwrap();
    let id = libc::id_t::try_from(pid).unwrap();
    // Wait for it to exit without reaping it (WNOWAIT): a zombie now.
    loop {
        // SAFETY: an all-zero siginfo_t is a valid value for waitid to fill.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        let flags = libc::WEXITED | libc::WNOWAIT;
        // SAFETY: `info` is a live siginfo_t; `pid` is this test's unreaped child.
        let waited = unsafe { libc::waitid(libc::P_PID, id, &mut info, flags) };
        if waited == 0 {
            // SAFETY: waitid filled `info` for an exited child.
            let exited = unsafe { info.si_pid() };
            assert_eq!(exited, pid, "waitid names the child");
            break;
        }
        let error = std::io::Error::last_os_error();
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::Interrupted,
            "waitid {pid}: {error}"
        );
    }
    // SAFETY: signal 0 only probes a pid this test spawned and has not reaped.
    let probed = unsafe { libc::kill(pid, 0) };
    assert_eq!(probed, 0, "a zombie still takes signal 0");
    assert_eq!(process_state(pid), Some('Z'));
    assert!(process_gone(pid), "the zombie {pid} counts as gone");
    assert!(child.wait().unwrap().success());
    assert!(process_gone(pid), "the reaped child {pid} is gone");
}

/// Kills and reaps its child when dropped, so a failing test leaves no
/// `sleep` behind.
struct KillOnDrop(std::process::Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// #2442: a helper that is still running is never gone, however long the
/// test polls, and the failure names it as never signalled: the timeout
/// test still fails on a helper that survives.
#[test]
fn a_running_helper_is_not_gone() {
    let child = KillOnDrop(
        std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap(),
    );
    let pid = libc::pid_t::try_from(child.0.id()).unwrap();
    let budget = GoneBudget {
        polls: 20,
        wall: Duration::from_millis(200),
    };
    let started = std::time::Instant::now();
    let waited = wait_gone(pid, budget);
    let elapsed = started.elapsed();
    drop(child);
    let survivor = waited.expect_err("a running helper is not gone");
    assert!(survivor.starts_with("state S,"), "{survivor}");
    assert!(survivor.contains("wchan"), "{survivor}");
    assert!(
        elapsed >= budget.wall,
        "both budgets are spent: {elapsed:?}"
    );
    assert!(process_gone(pid), "the killed, reaped helper {pid} is gone");
}

/// Review round 1 (L3): a stale `WAYLAND_DISPLAY` makes `wl-paste` fail;
/// the read falls through to `xclip`.
#[test]
fn a_failing_wl_paste_falls_through_to_xclip() {
    let dir = tempfile::tempdir().unwrap();
    let wl = fake_tool(
        dir.path(),
        "wl-paste",
        &[(
            "--list-types",
            "echo 'Failed to connect to a Wayland server' >&2; exit 1",
        )],
    );
    let xclip = fake_tool(
        dir.path(),
        "xclip",
        &[
            (
                "-selection clipboard -t TARGETS -o",
                "printf 'UTF8_STRING\\n'",
            ),
            ("-selection clipboard -o", "printf 'via x11'"),
        ],
    );
    let both = SystemClipboard::with_tools(
        vec![
            (ClipboardTool::WlPaste, wl.clone()),
            (ClipboardTool::Xclip, xclip),
        ],
        QUICK,
    );
    assert_eq!(both.read(), ClipboardRead::Text("via x11".into()));
    // Review round 2 (L4): with no tool left to try, a tool that could not
    // run is a failure, never an empty clipboard.
    let alone = SystemClipboard::with_tools(vec![(ClipboardTool::WlPaste, wl)], QUICK);
    match alone.read() {
        ClipboardRead::Failed(reason) => {
            assert!(reason.starts_with("wl-paste: "), "{reason}");
            assert!(
                reason.contains("Failed to connect to a Wayland server"),
                "{reason}"
            );
        }
        other => panic!("expected the run failure, got {other:?}"),
    }
}

/// Review round 2 (L4): a tool that ran and found the clipboard empty says
/// so on stderr; that, and only that, is an empty clipboard.
#[test]
fn only_a_tool_that_says_the_clipboard_is_empty_makes_it_empty() {
    let dir = tempfile::tempdir().unwrap();
    let wl = fake_tool(
        dir.path(),
        "wl-paste",
        &[(
            "--list-types",
            "echo 'Failed to connect to a Wayland server' >&2; exit 1",
        )],
    );
    let xclip = fake_tool(
        dir.path(),
        "xclip",
        &[(
            "-selection clipboard -t TARGETS -o",
            "echo 'Error: target TARGETS not available' >&2; exit 1",
        )],
    );
    let both = SystemClipboard::with_tools(
        vec![(ClipboardTool::WlPaste, wl), (ClipboardTool::Xclip, xclip)],
        QUICK,
    );
    assert_eq!(both.read(), ClipboardRead::Empty);

    let no_display = fake_tool(
        dir.path(),
        "xclip-2",
        &[(
            "-selection clipboard -t TARGETS -o",
            "echo 'Error: Can'\\''t open display: :0' >&2; exit 1",
        )],
    );
    let alone = SystemClipboard::with_tools(vec![(ClipboardTool::Xclip, no_display)], QUICK);
    match alone.read() {
        ClipboardRead::Failed(reason) => assert!(reason.contains("open display"), "{reason}"),
        other => panic!("expected the run failure, got {other:?}"),
    }
}

/// Review round 1 (L4): text types match whatever their case.
#[test]
fn text_types_match_case_insensitively() {
    let dir = tempfile::tempdir().unwrap();
    let tool = fake_tool(
        dir.path(),
        "wl-paste",
        &[
            ("--list-types", "printf 'text/plain;charset=UTF-8\\n'"),
            ("--no-newline", "printf 'upper'"),
        ],
    );
    assert_eq!(wl_only(tool).read(), ClipboardRead::Text("upper".into()));
}

/// Review round 1 (L4): a clipboard of neither images nor text says so, and
/// what it holds; X11's own bookkeeping targets are not named.
#[test]
fn a_clipboard_of_neither_image_nor_text_names_what_it_holds() {
    let dir = tempfile::tempdir().unwrap();
    let tool = fake_tool(
        dir.path(),
        "wl-paste",
        &[(
            "--list-types",
            "printf 'text/uri-list\\nx-special/gnome-copied-files\\n'",
        )],
    );
    assert_eq!(
        wl_only(tool).read(),
        ClipboardRead::NotPasteable("text/uri-list, x-special/gnome-copied-files".into())
    );
    let xclip = fake_tool(
        dir.path(),
        "xclip",
        &[(
            "-selection clipboard -t TARGETS -o",
            "printf 'TARGETS\\nTIMESTAMP\\nMULTIPLE\\ntext/uri-list\\n'",
        )],
    );
    let clipboard = SystemClipboard::with_tools(vec![(ClipboardTool::Xclip, xclip)], QUICK);
    assert_eq!(
        clipboard.read(),
        ClipboardRead::NotPasteable("text/uri-list".into())
    );
}
