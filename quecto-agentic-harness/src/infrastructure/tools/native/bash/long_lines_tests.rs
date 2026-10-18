//! #2196: a line too long to read inline shows its start and end, and says
//! how much of it was left out; every other line is untouched.
use super::{CutLine, LINE_KEEP_BYTES, LINE_MAX_BYTES, cut_note, window_long_lines};

#[test]
fn short_lines_are_untouched() {
    for text in ["", "a", "a\nb", "a\nb\n", "a\r\nb\r\n", "\n\n"] {
        let windowed = window_long_lines(text);
        assert_eq!(windowed.text, text);
        assert!(windowed.cut.is_empty());
    }
    let at_cap = "x".repeat(LINE_MAX_BYTES);
    let windowed = window_long_lines(&at_cap);
    assert_eq!(windowed.text, at_cap, "a line at the cap is kept whole");
    assert!(windowed.cut.is_empty());
    // Ordinary output is not copied.
    assert!(matches!(windowed.text, std::borrow::Cow::Borrowed(_)));
    // The `\r` of a `\r\n` terminator is not part of the line's length.
    let crlf = format!("{at_cap}\r\n");
    assert!(window_long_lines(&crlf).cut.is_empty());
}

/// Beside a cut line, a line at the cap is still kept whole, with or
/// without a `\r\n` terminator.
#[test]
fn a_line_at_the_cap_beside_a_cut_one_is_kept_whole() {
    let at_cap = "x".repeat(LINE_MAX_BYTES);
    let over = "y".repeat(LINE_MAX_BYTES + 1);
    let text = format!("{at_cap}\n{over}\n{at_cap}\r\n");
    let windowed = window_long_lines(&text);
    let numbers: Vec<usize> = windowed.cut.iter().map(|cut| cut.number).collect();
    assert_eq!(numbers, vec![2]);
    assert!(windowed.text.starts_with(&format!("{at_cap}\n")));
    assert!(windowed.text.ends_with(&format!("\n{at_cap}\r\n")));
}

#[test]
fn a_line_over_the_cap_keeps_its_start_and_end() {
    let line = format!(
        "{}{}{}",
        "a".repeat(LINE_KEEP_BYTES),
        "m".repeat(60_000),
        "z".repeat(LINE_KEEP_BYTES)
    );
    let windowed = window_long_lines(&line);
    let omitted = line.len() - 2 * LINE_KEEP_BYTES;
    assert_eq!(
        windowed.text,
        format!(
            "{}[... {omitted} bytes of line 1 omitted ...]{}",
            "a".repeat(LINE_KEEP_BYTES),
            "z".repeat(LINE_KEEP_BYTES)
        )
    );
    assert_eq!(
        windowed.cut,
        vec![CutLine {
            number: 1,
            bytes: line.len()
        }]
    );
}

#[test]
fn one_byte_over_the_cap_is_cut() {
    let line = "x".repeat(LINE_MAX_BYTES + 1);
    let windowed = window_long_lines(&line);
    assert_eq!(windowed.cut.len(), 1);
    assert!(
        windowed.text.len() < LINE_MAX_BYTES,
        "{}",
        windowed.text.len()
    );
}

/// Line numbers count as the tail's do; terminators, `\r\n` included, and
/// the lines around a cut one are kept byte for byte.
#[test]
fn only_the_long_line_changes_and_lines_keep_their_numbers() {
    let long = "q".repeat(LINE_MAX_BYTES * 2);
    let text = format!("first\r\n{long}\r\nthird\n{long}");
    let windowed = window_long_lines(&text);
    let numbers: Vec<usize> = windowed.cut.iter().map(|cut| cut.number).collect();
    assert_eq!(numbers, vec![2, 4]);
    assert_eq!(windowed.text.lines().count(), text.lines().count());
    let lines: Vec<&str> = windowed.text.lines().collect();
    assert_eq!(lines[0], "first");
    assert_eq!(lines[2], "third");
    assert!(lines[1].contains("of line 2 omitted"), "{}", lines[1]);
    assert!(lines[3].contains("of line 4 omitted"), "{}", lines[3]);
    assert!(windowed.text.contains("omitted ...]qq"));
    assert!(windowed.text.contains("\r\nthird\n"));
}

/// A cut never splits a character, and the bytes it moves are counted as
/// omitted.
#[test]
fn a_cut_never_splits_a_character() {
    for prefix in ["", "x", "xx"] {
        let line = format!("{prefix}{}", "日".repeat(LINE_MAX_BYTES));
        let windowed = window_long_lines(&line);
        assert!(!windowed.text.contains('\u{FFFD}'));
        let omitted: usize = windowed
            .text
            .split("[... ")
            .nth(1)
            .and_then(|rest| rest.split(' ').next())
            .and_then(|n| n.parse().ok())
            .unwrap();
        let marker = format!("[... {omitted} bytes of line 1 omitted ...]");
        assert_eq!(windowed.text.len() - marker.len() + omitted, line.len());
        let (head, tail) = windowed.text.split_once(&marker).unwrap();
        assert!(head.len() <= LINE_KEEP_BYTES && head.len() + 3 > LINE_KEEP_BYTES);
        assert!(tail.len() <= LINE_KEEP_BYTES && tail.len() + 3 > LINE_KEEP_BYTES);
    }
}

#[test]
fn the_note_names_a_single_cut_line_and_its_size() {
    let note = cut_note(&[CutLine {
        number: 1,
        bytes: 60_000,
    }]);
    assert_eq!(
        note,
        "line 1 is 60000 bytes, of which the first and last 2KB are shown"
    );
}

#[test]
fn the_note_counts_several_cut_lines() {
    let cuts = [
        CutLine {
            number: 2,
            bytes: 9000,
        },
        CutLine {
            number: 7,
            bytes: 12_000,
        },
    ];
    assert_eq!(
        cut_note(&cuts),
        "2 lines over 8KB (the longest 12000 bytes) show only their first and last 2KB"
    );
}
