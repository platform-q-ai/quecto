//! Contract for quecto-tui's `ClipboardReader` port (#2425): what every
//! reader the TUI can be given must do, checked against each one it ships —
//! the default `NoClipboard`, the system adapter (over fake `wl-paste` /
//! `xclip` executables, never the real clipboard) and the harness's fake.
//!
//! 1. A read is repeatable: with the clipboard unchanged, it answers the same.
//! 2. An image is never more than one byte past `MAX_IMAGE_BYTES`: the
//!    over-limit byte tells admission to refuse it, the rest is never read.
//! 3. A read never prints an image's bytes (`Debug`).
//! 4. A read that cannot run says so (`NoTool` / `Failed`); it never answers
//!    an empty clipboard for a tool it could not run.

use quecto_tui::shell::app::tui_harness::FakeClipboard;
use quecto_tui::shell::clipboard_image::{
    ClipboardRead, ClipboardReader, ClipboardTool, NoClipboard, SystemClipboard,
};
use quecto_tui::shell::test_executable::write_executable;
use std::path::Path;
use std::time::Duration;

fn assert_contract(reader: &dyn ClipboardReader, expected: &ClipboardRead) {
    let first = reader.read();
    assert_eq!(
        &first, expected,
        "the reader answers what the clipboard holds"
    );
    assert_eq!(reader.read(), first, "a read is repeatable");
    if let ClipboardRead::Image(bytes) = &first {
        assert!(
            bytes.len() <= quecto_image::MAX_IMAGE_BYTES + 1,
            "{}",
            bytes.len()
        );
        let debug = format!("{first:?}");
        assert!(debug.len() < 64, "a read never prints its bytes: {debug}");
    }
}

fn fake_tool(dir: &Path, name: &str, cases: &[(&str, &str)]) -> std::path::PathBuf {
    let mut script = String::from("#!/bin/sh\ncase \"$*\" in\n");
    for (args, answer) in cases {
        script.push_str(&format!("  \"{args}\") {answer} ;;\n"));
    }
    script.push_str("  *) exit 64 ;;\nesac\n");
    let path = dir.join(name);
    write_executable(&path, script);
    path
}

#[test]
fn no_clipboard_has_no_tool() {
    assert_contract(&NoClipboard, &ClipboardRead::NoTool);
}

#[test]
fn the_fake_answers_what_it_holds() {
    let png = quecto_image::samples::png(2, 2);
    for held in [
        ClipboardRead::Image(png),
        ClipboardRead::Text("words".into()),
        ClipboardRead::Empty,
        ClipboardRead::Failed("broken".into()),
    ] {
        assert_contract(&FakeClipboard(held.clone()), &held);
    }
}

#[test]
fn the_system_clipboard_honours_the_contract() {
    let dir = tempfile::tempdir().unwrap();
    let png = quecto_image::samples::png(3, 3);
    let image = dir.path().join("clip.png");
    std::fs::write(&image, &png).unwrap();
    let cat = format!("exec cat '{}'", image.display());
    let wl = fake_tool(
        dir.path(),
        "wl-paste",
        &[
            ("--list-types", "printf 'image/png\\n'"),
            ("--type image/png", &cat),
        ],
    );
    let quick = Duration::from_secs(5);
    let reader = SystemClipboard::with_tools(vec![(ClipboardTool::WlPaste, wl)], quick);
    assert_contract(&reader, &ClipboardRead::Image(png));

    let huge = fake_tool(
        dir.path(),
        "wl-paste-huge",
        &[
            ("--list-types", "printf 'image/png\\n'"),
            ("--type image/png", "exec head -c 5000000 /dev/zero"),
        ],
    );
    let reader = SystemClipboard::with_tools(vec![(ClipboardTool::WlPaste, huge)], quick);
    match reader.read() {
        ClipboardRead::Image(bytes) => {
            assert_eq!(bytes.len(), quecto_image::MAX_IMAGE_BYTES + 1)
        }
        other => panic!("expected the capped bytes, got {other:?}"),
    }

    let missing = dir.path().join("not-installed");
    let reader = SystemClipboard::with_tools(vec![(ClipboardTool::WlPaste, missing)], quick);
    assert_contract(&reader, &ClipboardRead::NoTool);
}
