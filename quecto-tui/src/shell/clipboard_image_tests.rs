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
