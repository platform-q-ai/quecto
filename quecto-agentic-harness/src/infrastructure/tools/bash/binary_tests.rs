//! #2197: output that is not text is named — by stream and likely cause —
//! not decoded into replacement characters; text with a few stray bytes, or
//! NUL-separated records, stays text.
use super::{Cause, Content, FEW_SUSPECT_BYTES, binary_notice, classify};
use crate::infrastructure::tools::bash::capture::Stream;

/// Deterministic noise shaped like `/dev/urandom` output.
fn noise(len: usize) -> Vec<u8> {
    let mut state: u32 = 0x2197_2197;
    (0..len)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 24) as u8
        })
        .collect()
}

fn is_binary(bytes: &[u8]) -> bool {
    matches!(classify(bytes), Content::Binary(_))
}

#[test]
fn random_bytes_are_binary_through_invalid_utf8() {
    assert_eq!(classify(&noise(500)), Content::Binary(Cause::InvalidUtf8));
    assert!(is_binary(&noise(64 * 1024)));
}

#[test]
fn runs_of_nuls_are_binary() {
    assert_eq!(classify(&[0u8; 500]), Content::Binary(Cause::NulHeavy));
}

/// UTF-16 text is valid UTF-8 byte by byte, but every other byte is a NUL
/// after a single byte.
#[test]
fn utf16_text_is_nul_heavy() {
    let utf16: Vec<u8> = "hello world, in UTF-16"
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    assert_eq!(classify(&utf16), Content::Binary(Cause::NulHeavy));
}

/// An ELF header and its padding: NUL runs and high bytes.
#[test]
fn an_elf_file_is_binary() {
    let mut elf = b"\x7fELF\x02\x01\x01\0\0\0\0\0\0\0\0\0\x03\0\x3e\0\x01\0\0\0".to_vec();
    elf.extend([0u8; 40]);
    elf.extend(b"\x40\0\0\0\0\0\0\0\xe8\x86\x01\0\0\0\0\0");
    assert!(is_binary(&elf));
}

#[test]
fn text_is_not_binary() {
    assert!(!is_binary(b""));
    assert!(!is_binary("héllo ✓\n".as_bytes()));
    assert!(!is_binary("日本語のテキスト\n".repeat(100).as_bytes()));
    assert!(!is_binary(b"\x1b[31mred\x1b[0m\ttab\r\n"));
}

/// `find -print0` and `xargs -0` input: NUL-separated names are text, short
/// ones included.
#[test]
fn nul_separated_names_are_text() {
    let names = "src/infrastructure/tools/bash/mod.rs\0".repeat(50);
    assert!(!is_binary(names.as_bytes()));
    let short = "./a\0./b\0./c\0./dd\0".repeat(20);
    assert!(!is_binary(short.as_bytes()));
    assert!(!is_binary(b"hi\0there\0"));
    assert!(!is_binary("hi\0there\0".repeat(40).as_bytes()));
}

/// A NUL ends a record only after two or more other bytes: single-byte
/// records are named as binary, an accepted limit that keeps UTF-16 so.
#[test]
fn a_nul_after_one_byte_is_suspect() {
    assert!(is_binary("a\0".repeat(20).as_bytes()));
    assert!(!is_binary("ab\0".repeat(20).as_bytes()));
}

/// Latin-1 text, or a line with one stray byte, keeps its text.
#[test]
fn a_few_invalid_bytes_keep_text_as_text() {
    assert!(!is_binary(b"ok\xff!"));
    let latin1 = b"caf\xe9 cr\xe8me br\xfbl\xe9e, a French dessert served cold. ".repeat(20);
    assert!(!is_binary(&latin1));
}

/// The rule's two edges: more than a few suspect bytes, and more than a
/// tenth of the output.
#[test]
fn the_threshold_needs_both_more_than_a_few_and_more_than_a_tenth() {
    // All suspect, but only a few: text.
    assert!(!is_binary(&[0u8; FEW_SUSPECT_BYTES]));
    assert!(is_binary(&[0u8; FEW_SUSPECT_BYTES + 1]));
    // Exactly a tenth is still text; one more suspect byte is binary.
    let mut tenth = vec![0u8; 10];
    tenth.extend([b'a'; 90]);
    assert!(!is_binary(&tenth));
    let mut over = vec![0u8; 11];
    over.extend([b'a'; 89]);
    assert!(is_binary(&over));
}

/// Invalid UTF-8 counts byte by byte, as NULs do, and both kinds add up.
#[test]
fn invalid_sequences_count_each_byte() {
    let mut bytes = vec![b'a'; 90];
    bytes.extend([0xFFu8; 11]);
    assert_eq!(classify(&bytes), Content::Binary(Cause::InvalidUtf8));
    let mut bytes = vec![b'a'; 100];
    bytes.extend([0xFFu8; 10]);
    assert!(!is_binary(&bytes));
    let mut mixed = vec![0u8; 6];
    mixed.extend([0xFFu8; 5]);
    mixed.extend([b'a'; 89]);
    assert_eq!(classify(&mixed), Content::Binary(Cause::NulHeavy));
    // A tie is named as invalid UTF-8: NUL-heavy needs more NULs.
    let mut tie = vec![0u8; 6];
    tie.extend([0xFFu8; 6]);
    tie.extend([b'a'; 88]);
    assert_eq!(classify(&tie), Content::Binary(Cause::InvalidUtf8));
}

#[test]
fn the_invalid_utf8_notice_names_the_stream_size_and_od_first() {
    let notice = binary_notice(500, Stream::Stdout, Cause::InvalidUtf8);
    assert!(
        notice.starts_with("[binary output on stdout (500 bytes, not UTF-8 text)"),
        "{notice}"
    );
    assert!(
        notice.contains("for Latin-1 text, through `iconv -f latin1 -t utf-8`"),
        "{notice}"
    );
    let od = notice.find("od -c").unwrap();
    assert!(od < notice.find("xxd").unwrap(), "{notice}");
    assert!(notice.contains("output_file"), "{notice}");
    assert!(!notice.contains('\n'), "one line: {notice}");
}

#[test]
fn the_nul_heavy_notice_suggests_iconv() {
    let notice = binary_notice(300, Stream::Stderr, Cause::NulHeavy);
    assert!(
        notice.starts_with("[binary output on stderr (300 bytes, mostly NUL"),
        "{notice}"
    );
    assert!(notice.contains("iconv -f UTF-16"), "{notice}");
    assert!(notice.contains("od -c"), "{notice}");
    assert!(!notice.contains('\n'), "one line: {notice}");
}

// --- through the capture and the tool ---

use crate::application::tools::ports::Tool;
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::bash::ExecTool;
use crate::infrastructure::tools::bash::capture::Capture;
use std::path::PathBuf;
use std::sync::Arc;

fn exec() -> (ExecTool, tempfile::TempDir) {
    let tmp = tempfile::TempDir::new().unwrap();
    let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()));
    let tool = ExecTool::new(Arc::new(PathBuf::from(tmp.path())), Arc::new(sandbox));
    (tool, tmp)
}

#[test]
fn a_binary_capture_renders_as_its_notice() {
    let mut capture = Capture::new(1024 * 1024, Stream::Stderr);
    capture.push(&noise(500));
    let (rendered, cut) = capture.render();
    assert_eq!(
        rendered,
        binary_notice(500, Stream::Stderr, Cause::InvalidUtf8)
    );
    assert!(!cut);
}

/// A binary stream past the capture cap is named with its whole size.
#[test]
fn a_cut_binary_capture_names_the_whole_stream() {
    let mut capture = Capture::new(1000, Stream::Stdout);
    capture.push(&noise(5000));
    let (rendered, cut) = capture.render();
    assert_eq!(
        rendered,
        binary_notice(5000, Stream::Stdout, Cause::InvalidUtf8)
    );
    assert!(cut);
}

/// Text past the capture cap is still text, with its middle named.
#[test]
fn a_cut_text_capture_stays_text() {
    let mut capture = Capture::new(1000, Stream::Stdout);
    capture.push("line of text\n".repeat(500).as_bytes());
    let (rendered, cut) = capture.render();
    assert!(cut);
    assert!(rendered.contains("bytes of output omitted"), "{rendered}");
    assert!(!rendered.contains("binary output"), "{rendered}");
}

#[tokio::test]
async fn random_bytes_on_stdout_come_back_as_a_notice() {
    let (tool, _tmp) = exec();
    let result = tool
        .execute(r#"{"command": "head -c 500 /dev/urandom"}"#)
        .await
        .unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(
        result.content,
        binary_notice(500, Stream::Stdout, Cause::InvalidUtf8)
    );
}

/// The streams are judged apart: text on stderr is kept beside a binary
/// stdout, and binary on stderr is named as stderr's.
#[tokio::test]
async fn each_stream_is_judged_on_its_own() {
    let (tool, _tmp) = exec();
    let result = tool
        .execute(r#"{"command": "head -c 500 /dev/urandom; echo 'warning: done' >&2"}"#)
        .await
        .unwrap();
    assert_eq!(
        result.content,
        format!(
            "{}\nwarning: done",
            binary_notice(500, Stream::Stdout, Cause::InvalidUtf8)
        )
    );
    let result = tool
        .execute(r#"{"command": "echo listing; head -c 300 /dev/zero >&2"}"#)
        .await
        .unwrap();
    assert_eq!(
        result.content,
        format!(
            "listing\n\n{}",
            binary_notice(300, Stream::Stderr, Cause::NulHeavy)
        )
    );
}

#[tokio::test]
async fn unicode_and_nul_separated_text_are_unchanged() {
    let (tool, _tmp) = exec();
    let result = tool
        .execute(r#"{"command": "printf 'h\\303\\251llo \\342\\234\\223\\n'"}"#)
        .await
        .unwrap();
    assert_eq!(result.content, "héllo ✓");
    let result = tool
        .execute(r#"{"command": "printf 'hi\\0there\\0'"}"#)
        .await
        .unwrap();
    assert_eq!(result.content, "hi\0there\0");
}
