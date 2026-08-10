use super::line_cap::*;

#[test]
fn event_caps_derive_from_shared_line_io_contract() {
    assert_eq!(
        EVENT_LINE_CAP_BYTES,
        quecto_line_io::PROTOCOL_LINE_CAP_BYTES
    );
    assert_eq!(EVENT_LINE_JSON_BUDGET, EVENT_LINE_CAP_BYTES - 1);
}
