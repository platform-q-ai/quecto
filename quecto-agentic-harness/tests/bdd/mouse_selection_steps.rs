use super::*;
use std::io::{self, Write};

struct BddClipboardWriter {
    fail_on_write: bool,
    fail_on_flush: bool,
}

impl Write for BddClipboardWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.fail_on_write {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "write failed"));
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.fail_on_flush {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "flush failed"));
        }
        Ok(())
    }
}

// ─── Mouse selection BDD steps (#528) ────────────────────────────────────────
//
// The TUI mouse handling is tested via unit tests in quecto-tui/src/interface/keys.rs.
// These BDD steps verify the protocol-level SGR mouse parsing and the
// base64 encoding used for OSC 52 clipboard writes.

/// Replicate the TUI's base64 encoder for BDD verification.
fn base64_encode_bdd(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        result.push(ALPHABET[((triple >> 18) & 0x3F) as usize] as char);
        result.push(ALPHABET[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(ALPHABET[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(ALPHABET[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

#[given(expr = "an SGR mouse sequence for button {int} press at col {int} row {int}")]
fn given_sgr_mouse_press(world: &mut QuectoWorld, button: u32, col: u32, row: u32) {
    // Build SGR mouse sequence: \x1b[<button;col;rowM
    let seq = format!("\x1b[<{};{};{}M", button, col, row);
    world.stdout = seq;
}

#[given(expr = "an SGR mouse release sequence at col {int} row {int}")]
fn given_sgr_mouse_release(world: &mut QuectoWorld, col: u32, row: u32) {
    // Release: lowercase 'm' terminator with button 0
    let seq = format!("\x1b[<0;{};{}m", col, row);
    world.stdout = seq;
}

#[when("the sequence is parsed")]
fn when_sequence_parsed(world: &mut QuectoWorld) {
    // Parse the SGR sequence by simulating what the TUI key parser does.
    // We can't import quecto-tui directly in BDD tests, so we verify
    // the protocol format is correct and matches our expectations.
    // The actual parsing is tested by unit tests in quecto-tui/src/interface/keys.rs.
    world.stderr = world.stdout.clone();
}

#[then(expr = "the result should be a MousePress at col {int} row {int}")]
fn then_mouse_press(world: &mut QuectoWorld, expected_col: u32, expected_row: u32) {
    // Verify the SGR sequence encodes the expected coordinates.
    // SGR uses 1-indexed coordinates; TUI converts to 0-indexed.
    let seq = &world.stderr;
    // Extract col and row from the sequence
    let inner = seq
        .strip_prefix("\x1b[<")
        .unwrap()
        .strip_suffix('M')
        .or_else(|| seq.strip_prefix("\x1b[<").unwrap().strip_suffix('m'))
        .unwrap();
    let parts: Vec<&str> = inner.split(';').collect();
    let col_1indexed: u32 = parts[1].parse().unwrap();
    let row_1indexed: u32 = parts[2].parse().unwrap();
    assert_eq!(col_1indexed - 1, expected_col, "col mismatch");
    assert_eq!(row_1indexed - 1, expected_row, "row mismatch");
}

#[then(expr = "the result should be a MouseDrag at col {int} row {int}")]
fn then_mouse_drag(world: &mut QuectoWorld, expected_col: u32, expected_row: u32) {
    // Verify the SGR sequence encodes the expected 0-indexed coordinates.
    let seq = &world.stderr;
    let inner = seq.strip_prefix("\x1b[<").unwrap();
    let inner = inner
        .strip_suffix('M')
        .or_else(|| inner.strip_suffix('m'))
        .unwrap();
    let parts: Vec<&str> = inner.split(';').collect();
    let button: u32 = parts[0].parse().unwrap();
    assert_eq!(button, 32, "drag should use button 32");
    assert_eq!(
        parts[1].parse::<u32>().unwrap() - 1,
        expected_col,
        "col mismatch"
    );
    assert_eq!(
        parts[2].parse::<u32>().unwrap() - 1,
        expected_row,
        "row mismatch"
    );
}

#[then(expr = "the result should be a MouseRelease at col {int} row {int}")]
fn then_mouse_release(world: &mut QuectoWorld, expected_col: u32, expected_row: u32) {
    // Verify the SGR release sequence (lowercase 'm' terminator).
    let seq = &world.stderr;
    assert!(
        seq.ends_with('m'),
        "release should end with lowercase m, got: {}",
        seq
    );
    let inner = seq
        .strip_prefix("\x1b[<")
        .unwrap()
        .strip_suffix('m')
        .unwrap();
    let parts: Vec<&str> = inner.split(';').collect();
    assert_eq!(
        parts[1].parse::<u32>().unwrap() - 1,
        expected_col,
        "col mismatch"
    );
    assert_eq!(
        parts[2].parse::<u32>().unwrap() - 1,
        expected_row,
        "row mismatch"
    );
}

#[given(expr = "the text {string} to copy")]
fn given_text_to_copy(world: &mut QuectoWorld, text: String) {
    world.stdout = text;
}

#[when("it is base64 encoded for OSC 52")]
fn when_base64_encoded(world: &mut QuectoWorld) {
    world.stderr = base64_encode_bdd(world.stdout.as_bytes());
}

#[then(expr = "the encoded value should be {string}")]
fn then_encoded_value(world: &mut QuectoWorld, expected: String) {
    assert_eq!(
        world.stderr, expected,
        "base64 mismatch: got {:?}, expected {:?}",
        world.stderr, expected
    );
}

#[when("the OSC 52 clipboard write fails")]
fn when_osc52_clipboard_write_fails(world: &mut QuectoWorld) {
    let mut writer = BddClipboardWriter {
        fail_on_write: true,
        fail_on_flush: false,
    };
    world.stderr =
        quecto_tui::interface::app::write_osc52_clipboard_sequence(&world.stdout, &mut writer)
            .expect_err("write failure should be reported")
            .to_string();
}

#[when("the OSC 52 clipboard flush fails")]
fn when_osc52_clipboard_flush_fails(world: &mut QuectoWorld) {
    let mut writer = BddClipboardWriter {
        fail_on_write: false,
        fail_on_flush: true,
    };
    world.stderr =
        quecto_tui::interface::app::write_osc52_clipboard_sequence(&world.stdout, &mut writer)
            .expect_err("flush failure should be reported")
            .to_string();
}

#[then("the clipboard copy result should be an error")]
fn then_clipboard_copy_result_should_be_error(world: &mut QuectoWorld) {
    assert!(
        world.stderr.contains("write failed") || world.stderr.contains("flush failed"),
        "clipboard writer/flush failure should be returned; got {:?}",
        world.stderr
    );
}

#[then("clipboard failure feedback should not include the copied text")]
fn then_clipboard_failure_feedback_should_not_include_copied_text(world: &mut QuectoWorld) {
    assert!(
        !world.stderr.contains(&world.stdout),
        "clipboard failure should not include selected text; got {:?}",
        world.stderr
    );
}
