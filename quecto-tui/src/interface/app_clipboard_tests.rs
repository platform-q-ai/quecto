use std::io::{self, Write};

struct FailingClipboardWriter {
    fail_on_write: bool,
    fail_on_flush: bool,
    bytes: Vec<u8>,
}

impl FailingClipboardWriter {
    fn success() -> Self {
        Self {
            fail_on_write: false,
            fail_on_flush: false,
            bytes: Vec::new(),
        }
    }

    fn write_failure() -> Self {
        Self {
            fail_on_write: true,
            fail_on_flush: false,
            bytes: Vec::new(),
        }
    }

    fn flush_failure() -> Self {
        Self {
            fail_on_write: false,
            fail_on_flush: true,
            bytes: Vec::new(),
        }
    }
}

impl Write for FailingClipboardWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.fail_on_write {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "write failed"));
        }
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.fail_on_flush {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "flush failed"));
        }
        Ok(())
    }
}

#[test]
fn write_osc52_clipboard_sequence_writes_and_flushes() {
    let mut writer = FailingClipboardWriter::success();

    super::write_osc52_clipboard_sequence("hello world", &mut writer)
        .expect("clipboard write should succeed");

    assert_eq!(
        String::from_utf8(writer.bytes).expect("OSC52 bytes should be UTF-8"),
        "\x1b]52;c;aGVsbG8gd29ybGQ=\x07"
    );
}

#[test]
fn write_osc52_clipboard_sequence_returns_write_failure() {
    let mut writer = FailingClipboardWriter::write_failure();

    let err = super::write_osc52_clipboard_sequence("secret selected text", &mut writer)
        .expect_err("write failure should be returned");

    assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    assert!(err.to_string().contains("write failed"));
}

#[test]
fn write_osc52_clipboard_sequence_returns_flush_failure() {
    let mut writer = FailingClipboardWriter::flush_failure();

    let err = super::write_osc52_clipboard_sequence("secret selected text", &mut writer)
        .expect_err("flush failure should be returned");

    assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    assert!(err.to_string().contains("flush failed"));
}
