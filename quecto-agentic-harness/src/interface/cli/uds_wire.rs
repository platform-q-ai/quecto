//! Per-connection wire-mode negotiation for the UDS protocol
//! (ADR-0008 part 1, #1059).
//!
//! Each connection's framing is sniffed from the first byte of the first
//! message the client sends (`quecto_line_io::read_frame_or_legacy_line`).
//! The reader records the detected [`WireMode`] here; every writer for the
//! same connection then replies in that framing. Until the client has spoken
//! (or when a `{`-opening legacy line is seen) writes fall back to legacy
//! NDJSON — safe during the deprecation window because framed clients sniff
//! each incoming message, while legacy clients only ever receive lines.

use quecto_line_io::{PROTOCOL_FRAME_CAP_BYTES, PROTOCOL_VERSION, WireMode, write_message};

// 0 (`AtomicU8::default()`) = not yet negotiated.
const MODE_FRAMED: u8 = 1;
const MODE_LEGACY: u8 = 2;

/// Shared, cheaply-cloneable record of one connection's negotiated framing.
/// Set once by the connection's reader; read by its writer(s).
#[derive(Clone, Debug, Default)]
pub(crate) struct ConnectionWireMode(std::sync::Arc<std::sync::atomic::AtomicU8>);

impl ConnectionWireMode {
    /// A handle pre-pinned to legacy NDJSON, for writers that never negotiate
    /// (e.g. the plain-stdout agent path).
    pub(crate) fn legacy() -> Self {
        let mode = Self::default();
        mode.record(WireMode::LegacyLine);
        mode
    }

    /// Record the framing detected by the connection's reader.
    pub(crate) fn record(&self, mode: WireMode) {
        let v = match mode {
            WireMode::Framed => MODE_FRAMED,
            WireMode::LegacyLine => MODE_LEGACY,
        };
        self.0.store(v, std::sync::atomic::Ordering::SeqCst);
    }

    /// The framing to write in: the negotiated mode, or legacy NDJSON while
    /// the client has not spoken yet (see module docs for why that is safe).
    pub(crate) fn effective(&self) -> WireMode {
        match self.0.load(std::sync::atomic::Ordering::SeqCst) {
            MODE_FRAMED => WireMode::Framed,
            _ => WireMode::LegacyLine,
        }
    }
}

/// Write one serialized event line (WITH its trailing `\n`, the historical
/// emit convention) in the connection's negotiated framing. For framed
/// connections the newline is stripped and the JSON payload is sent as one
/// length-prefixed frame.
pub(crate) async fn write_event_line<W>(
    writer: &mut W,
    line: &str,
    mode: &ConnectionWireMode,
) -> Result<(), quecto_line_io::FrameError>
where
    W: tokio::io::AsyncWrite + Unpin + ?Sized,
{
    let payload = line.strip_suffix('\n').unwrap_or(line).as_bytes();
    write_message(writer, payload, mode.effective(), PROTOCOL_FRAME_CAP_BYTES).await
}

/// The stderr announcement a starting UDS agent prints: a protocol-version
/// token line followed by the socket-path line. The version line comes FIRST
/// so a client knows the framing to speak before it connects; it is a
/// separate line so pre-#1059 clients (which parse only the
/// `quecto-agent-socket: ` prefix) keep working through the deprecation
/// window.
pub fn socket_announcement(socket_path: &std::path::Path) -> String {
    format!(
        "{PROTOCOL_ANNOUNCE_PREFIX}{PROTOCOL_VERSION}\nquecto-agent-socket: {}\n",
        socket_path.display()
    )
}

/// Prefix of the protocol-version announcement line.
pub const PROTOCOL_ANNOUNCE_PREFIX: &str = "quecto-agent-protocol: ";
