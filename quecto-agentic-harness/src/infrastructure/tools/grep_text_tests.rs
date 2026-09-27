//! #2251 review (M2): a line that is not UTF-8 is decoded lossily, and rg's
//! offsets (into its raw bytes) are translated both ways.
use super::*;

#[test]
fn valid_text_maps_offsets_to_themselves() {
    let line = Decoded::new("héllo wörld".as_bytes());
    assert_eq!(line.text, "héllo wörld");
    assert_eq!(line.raw_len(), "héllo wörld".len());
    for at in 0..=line.text.len() {
        assert_eq!(line.shown_at(at), at);
    }
    for at in (0..=line.text.len()).filter(|at| line.text.is_char_boundary(*at)) {
        assert_eq!(line.raw_at(at), at);
    }
}

#[test]
fn each_invalid_byte_becomes_one_replacement_and_offsets_follow_it() {
    // 400 invalid bytes, then `needle`: raw 400 is decoded 1200.
    let mut raw = vec![0xE9; 400];
    raw.extend_from_slice(b"needle");
    let line = Decoded::new(&raw);
    assert_eq!(line.text.len(), 400 * 3 + 6);
    assert_eq!(line.raw_len(), 406);
    assert_eq!(line.shown_at(400), 1200);
    assert_eq!(&line.text[line.shown_at(400)..line.shown_at(406)], "needle");
    assert_eq!(line.raw_at(1200), 400);
    assert_eq!(line.raw_at(line.text.len()), 406);
    // Every replacement, both edges, and inside each U+FFFD: the start of
    // the byte it stands for.
    for byte in 0..400 {
        assert_eq!(line.shown_at(byte), byte * 3);
        assert_eq!(line.raw_at(byte * 3), byte);
        assert_eq!(line.raw_at(byte * 3 + 1), byte);
        assert_eq!(line.raw_at(byte * 3 + 2), byte);
    }
}

#[test]
fn a_truncated_sequence_is_one_replacement_for_all_its_bytes() {
    // `\xE2\x82` is the start of a 3-byte character cut short: one U+FFFD
    // for 2 raw bytes; then `x`, a valid 2-byte `é`, an invalid byte, `y`.
    let raw = b"a\xE2\x82x\xC3\xA9\xFFy";
    let line = Decoded::new(raw);
    assert_eq!(line.text, "a\u{FFFD}xé\u{FFFD}y");
    assert_eq!(line.raw_len(), raw.len());
    let pairs = [(0, 0), (1, 1), (3, 4), (4, 5), (6, 7), (7, 10), (8, 11)];
    for (raw_at, decoded_at) in pairs {
        assert_eq!(line.shown_at(raw_at), decoded_at, "raw {raw_at}");
        assert_eq!(line.raw_at(decoded_at), raw_at, "decoded {decoded_at}");
    }
    // Inside the cut sequence: its replacement's start.
    assert_eq!(line.shown_at(2), 1);
    // Past the raw bytes: the text's end.
    assert_eq!(line.shown_at(99), line.text.len());
}

#[test]
fn a_literal_replacement_character_is_valid_text() {
    let line = Decoded::new("a\u{FFFD}b".as_bytes());
    assert_eq!(line.raw_len(), 5);
    assert_eq!(line.shown_at(4), 4);
    assert_eq!(line.raw_at(4), 4);
}

#[test]
fn reported_lines_are_split_and_decoded_one_by_one() {
    let block = b"one\xE9\r\ntwo\n";
    let lines = reported_lines(Some(block)).unwrap();
    assert_eq!(
        lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(),
        ["one\u{FFFD}", "two"]
    );
    assert_eq!(lines[0].raw_len(), 4);
}

#[test]
fn cached_lines_split_as_before_and_keep_raw_offsets() {
    let lines = split_lines(b"a\xE9b\r\nc\rd\n\ne");
    assert_eq!(
        lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(),
        ["a\u{FFFD}b", "c", "d", "", "e"]
    );
    assert_eq!(lines[0].raw_len(), 3);
    assert_eq!(lines[0].raw_at(4), 2);
    assert!(split_lines(b"").is_empty());
    assert_eq!(split_lines(b"x\n").len(), 1);
    assert_eq!(split_lines(b"\n").len(), 1);
}
