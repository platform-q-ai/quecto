use super::*;

#[test]
fn max_line_bytes_matches_shared_line_io_cap() {
    assert_eq!(MAX_LINE_BYTES, quecto_line_io::PROTOCOL_LINE_CAP_BYTES);
}
