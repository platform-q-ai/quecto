//! Shared protocol event-line bound (#1062).
//!
//! Event schemas and history paging keep legitimate payloads below this bound.
//! An over-cap event is therefore rejected whole at the emitting boundary; it
//! must never be reshaped by silently dropping user-visible content.

/// Hard cap on one emitted event line, including its trailing newline.
///
/// This is derived from the shared framing bound, so emitters and every reader
/// agree on one authoritative value.
pub const EVENT_LINE_CAP_BYTES: usize = quecto_line_io::PROTOCOL_LINE_CAP_BYTES;

/// Serialized JSON budget before the emitter appends its trailing newline.
pub const EVENT_LINE_JSON_BUDGET: usize = EVENT_LINE_CAP_BYTES - 1;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_caps_derive_from_shared_line_io_contract() {
        assert_eq!(
            EVENT_LINE_CAP_BYTES,
            quecto_line_io::PROTOCOL_LINE_CAP_BYTES
        );
        assert_eq!(EVENT_LINE_JSON_BUDGET, EVENT_LINE_CAP_BYTES - 1);
    }
}
