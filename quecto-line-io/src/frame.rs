//! ADR-0008 part 1 (#1059): length-prefixed framing with version
//! negotiation for quecto's UDS protocol. Re-exported from the crate root so
//! consumers use `quecto_line_io::write_frame` etc.
//!
//! Wire layout (behaviour is the contract, not the byte layout — ADR-0010):
//! each frame is a 4-byte big-endian payload-byte-length prefix followed by
//! exactly that many bytes of UTF-8 JSON. Because the frame cap is far below
//! 2^24, the first byte of every legal frame is `0x00`, which is what the
//! deprecation-window sniff uses to tell a frame from a legacy NDJSON line
//! (`{` = 0x7B). All four socket consumers (TUI client, harness uds_reader,
//! sub-agent parent monitor, extension protocol reader) share these helpers.

use tokio::io::{AsyncBufRead, AsyncBufReadExt};

use crate::{INITIAL_CAPACITY, PROTOCOL_LINE_CAP_BYTES, RECLAIM_THRESHOLD, read_bounded_line_into};

/// Byte length of the frame's size prefix.
const FRAME_PREFIX_LEN: usize = 4;

/// The first byte of a legacy newline-framed JSON message (`{`), used by the
/// deprecation-window sniff in [`read_frame_or_legacy_line`].
const LEGACY_JSON_OPENER: u8 = b'{';

/// Single source of truth for the framed protocol's per-frame payload cap.
/// Same value and rationale as [`PROTOCOL_LINE_CAP_BYTES`] (the #1051 cap
/// era's value stays as-is per ADR-0008; the cap-as-invariant change is part 3).
pub const PROTOCOL_FRAME_CAP_BYTES: usize = PROTOCOL_LINE_CAP_BYTES;

/// Protocol version announced by the agent (in the `quecto-agent-socket:`
/// stderr announcement) so a client can act on it before speaking.
pub const PROTOCOL_VERSION: u8 = 2;

/// Prefix of the agent's protocol-version announcement line
/// (`quecto-agent-protocol: <N>`). The single source of truth shared by the
/// producer (the agent's stderr announcement) and every consumer that sniffs
/// it (the TUI spawn path). Hand-duplicating this literal in a consumer crate
/// reintroduces the silent-version-mismatch failure mode ADR-0008 forbids: a
/// later edit to one copy leaves the other silently pinned to legacy NDJSON.
pub const PROTOCOL_ANNOUNCE_PREFIX: &str = "quecto-agent-protocol: ";

/// Errors from the framed reader/writer, distinct from I/O errors so callers
/// can log a clean protocol error and keep the connection alive.
#[derive(Debug)]
pub enum FrameError {
    /// The peer declared a frame larger than `max`. The declared size was
    /// learned *before* buffering the payload; the reader consumes and
    /// discards the declared payload so subsequent frames still parse.
    Oversized { declared: usize, max: usize },
    /// The connection's first byte was neither a legacy NDJSON opener (`{`)
    /// nor a valid frame prefix: the peer speaks an unknown protocol version.
    VersionMismatch { first_byte: u8 },
    /// Underlying transport error.
    Io(std::io::Error),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Oversized { declared, max } => write!(
                f,
                "protocol error: peer declared a {declared}-byte frame, over the {max}-byte frame cap"
            ),
            Self::VersionMismatch { first_byte } => write!(
                f,
                "protocol version mismatch: first byte {first_byte:#04x} is neither a \
                 length-prefixed frame nor a legacy newline-delimited JSON line"
            ),
            Self::Io(e) => write!(f, "frame I/O error: {e}"),
        }
    }
}

impl std::error::Error for FrameError {}

impl From<std::io::Error> for FrameError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// One message read off a negotiating connection during the NDJSON
/// deprecation window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Incoming {
    /// A length-prefixed frame's payload (UTF-8 JSON bytes).
    Frame(Vec<u8>),
    /// A legacy `\n`-terminated NDJSON line's content (without the `\n`),
    /// accepted only for the deprecation window.
    LegacyLine(Vec<u8>),
}

/// Which framing a connection speaks. `LegacyLine` exists only for the
/// NDJSON deprecation window documented in ADR-0008.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireMode {
    /// Length-prefixed frames (protocol version [`PROTOCOL_VERSION`]).
    Framed,
    /// Legacy `\n`-terminated NDJSON lines (deprecation window only).
    LegacyLine,
}

/// Write one JSON message (`payload` excludes any trailing newline) in the
/// connection's negotiated framing: a length-prefixed frame, or — during the
/// NDJSON deprecation window — a `\n`-terminated legacy line.
pub async fn write_message<W>(
    writer: &mut W,
    payload: &[u8],
    mode: WireMode,
    max_bytes: usize,
) -> Result<(), FrameError>
where
    W: tokio::io::AsyncWrite + Unpin + ?Sized,
{
    use tokio::io::AsyncWriteExt;
    match mode {
        WireMode::Framed => write_frame(writer, payload, max_bytes).await,
        WireMode::LegacyLine => {
            // The legacy cap covers the whole wire line, including its newline
            // (the reader's PROTOCOL_LINE_CAP_BYTES convention).
            let declared = payload.len().checked_add(1).ok_or(FrameError::Oversized {
                declared: usize::MAX,
                max: max_bytes,
            })?;
            if declared > max_bytes {
                return Err(FrameError::Oversized {
                    declared,
                    max: max_bytes,
                });
            }
            writer.write_all(payload).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;
            Ok(())
        }
    }
}

/// Write `payload` as one length-prefixed frame. Refuses (with
/// [`FrameError::Oversized`]) to emit a frame larger than `max_bytes`, so an
/// emitter can never legally produce a frame a reader rejects. Nothing
/// reaches the wire on refusal.
pub async fn write_frame<W>(
    writer: &mut W,
    payload: &[u8],
    max_bytes: usize,
) -> Result<(), FrameError>
where
    W: tokio::io::AsyncWrite + Unpin + ?Sized,
{
    use tokio::io::AsyncWriteExt;
    if payload.len() > max_bytes {
        return Err(FrameError::Oversized {
            declared: payload.len(),
            max: max_bytes,
        });
    }
    let len = u32::try_from(payload.len()).map_err(|_| FrameError::Oversized {
        declared: payload.len(),
        max: max_bytes,
    })?;
    writer.write_all(&len.to_be_bytes()).await?;
    writer.write_all(payload).await?;
    writer.flush().await?;
    Ok(())
}

/// Fill `buf` with exactly `buf.len()` bytes from `reader`. Returns
/// `Ok(false)` on clean EOF before the first byte, `Err(UnexpectedEof)` on
/// EOF mid-way, `Ok(true)` when filled.
async fn read_exact_buffered<R>(reader: &mut R, buf: &mut [u8]) -> std::io::Result<bool>
where
    R: AsyncBufRead + Unpin,
{
    let mut filled = 0;
    while filled < buf.len() {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            if filled == 0 {
                return Ok(false);
            }
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        let n = (buf.len() - filled).min(available.len());
        buf[filled..filled + n].copy_from_slice(&available[..n]);
        reader.consume(n);
        filled += n;
    }
    Ok(true)
}

/// Consume and discard up to `len` bytes, stopping early at EOF. Bounded
/// memory: bytes pass through the reader's internal buffer only.
async fn discard_up_to<R>(reader: &mut R, mut len: usize) -> std::io::Result<()>
where
    R: AsyncBufRead + Unpin,
{
    while len > 0 {
        let available = reader.fill_buf().await?;
        if available.is_empty() {
            return Ok(());
        }
        let take = available.len().min(len);
        reader.consume(take);
        len -= take;
    }
    Ok(())
}

/// Read one length-prefixed frame. The size is known from the prefix *before*
/// the payload is buffered: an over-limit declaration returns
/// [`FrameError::Oversized`] without ever buffering the payload — the
/// declared bytes are consumed and discarded (bounded memory) so the stream
/// stays positioned at the next frame; if the peer withholds the payload and
/// closes, the rejection is still surfaced as `Oversized` (framing is moot at
/// EOF). Returns `Ok(None)` at clean EOF.
pub async fn read_frame<R>(reader: &mut R, max_bytes: usize) -> Result<Option<Vec<u8>>, FrameError>
where
    R: AsyncBufRead + Unpin,
{
    let mut prefix = [0u8; FRAME_PREFIX_LEN];
    if !read_exact_buffered(reader, &mut prefix).await? {
        return Ok(None);
    }
    let declared = u32::from_be_bytes(prefix) as usize;
    if declared > max_bytes {
        discard_up_to(reader, declared).await?;
        return Err(FrameError::Oversized {
            declared,
            max: max_bytes,
        });
    }
    let mut payload = vec![0u8; declared];
    if !read_exact_buffered(reader, &mut payload).await? {
        // EOF between a valid prefix and its payload: a broken peer, not a
        // clean close.
        return Err(FrameError::Io(std::io::ErrorKind::UnexpectedEof.into()));
    }
    Ok(Some(payload))
}

/// Deprecation-window reader: sniffs each message's framing from its first
/// byte (`{` = legacy NDJSON, `0x00` frame-prefix opener = framed, anything
/// else = [`FrameError::VersionMismatch`]) and then reads the message in the
/// detected framing. Never hangs or silently misparses on a mixed-version
/// peer: an unknown first byte is an explicit error, an over-cap legacy line
/// is consumed and rejected as [`FrameError::Oversized`] (with the observed
/// wire size as `declared`), and framed messages inherit [`read_frame`]'s
/// reject-before-buffering behaviour.
pub async fn read_frame_or_legacy_line<R>(
    reader: &mut R,
    max_bytes: usize,
) -> Result<Option<Incoming>, FrameError>
where
    R: AsyncBufRead + Unpin,
{
    let first_byte = loop {
        let available = reader.fill_buf().await?;
        match available.first() {
            // Blank legacy lines (`\n` / `\r\n` keep-alives) are no-ops, not
            // a foreign protocol: skip them before sniffing.
            Some(&(b'\n' | b'\r')) => reader.consume(1),
            Some(&b) => break b,
            None => return Ok(None),
        }
    };
    match first_byte {
        LEGACY_JSON_OPENER => {
            let mut line = Vec::new();
            // `max_bytes` bounds the whole wire line INCLUDING its trailing
            // newline (the `PROTOCOL_LINE_CAP_BYTES` convention), so a legal
            // emitter's capped line always fits.
            let Some(read) = read_bounded_line_into(reader, &mut line, max_bytes).await? else {
                return Ok(None);
            };
            if read.truncated {
                return Err(FrameError::Oversized {
                    declared: read.bytes_read,
                    max: max_bytes,
                });
            }
            if line.ends_with(b"\n") {
                line.pop();
            }
            Ok(Some(Incoming::LegacyLine(line)))
        }
        0x00 => Ok(read_frame(reader, max_bytes).await?.map(Incoming::Frame)),
        other => Err(FrameError::VersionMismatch { first_byte: other }),
    }
}

/// Buffer-reusing twin of [`read_frame_or_legacy_line`] for hot event-reader
/// loops (the TUI client, `quecto-api` gateway, harness readers) that read one
/// small JSON message per token: it sniffs and reads the next message into a
/// caller-owned `Vec<u8>` — reused across calls with the same `shrink_to`
/// reclaim as [`read_bounded_line_into`] — instead of allocating (and, for the
/// framed branch, zero-initializing) a fresh `Vec` per message. On success the
/// detected framing is returned and `buf` holds the message payload (no
/// trailing `\n`); the same [`FrameError::Oversized`] / [`FrameError::VersionMismatch`]
/// / clean-EOF (`Ok(None)`) semantics apply.
pub async fn read_frame_or_legacy_line_into<R>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    max_bytes: usize,
) -> Result<Option<WireMode>, FrameError>
where
    R: AsyncBufRead + Unpin,
{
    let first_byte = loop {
        let available = reader.fill_buf().await?;
        match available.first() {
            Some(&(b'\n' | b'\r')) => reader.consume(1),
            Some(&b) => break b,
            None => return Ok(None),
        }
    };
    match first_byte {
        LEGACY_JSON_OPENER => {
            let Some(read) = read_bounded_line_into(reader, buf, max_bytes).await? else {
                return Ok(None);
            };
            if read.truncated {
                return Err(FrameError::Oversized {
                    declared: read.bytes_read,
                    max: max_bytes,
                });
            }
            if buf.ends_with(b"\n") {
                buf.pop();
            }
            Ok(Some(WireMode::LegacyLine))
        }
        0x00 => match read_frame_into(reader, buf, max_bytes).await? {
            true => Ok(Some(WireMode::Framed)),
            false => Ok(None),
        },
        other => Err(FrameError::VersionMismatch { first_byte: other }),
    }
}

/// Buffer-reusing twin of [`read_frame`]. Fills `buf` with the next frame's
/// payload (reusing its allocation, with the same reclaim policy as
/// [`read_bounded_line_into`]) and returns `Ok(true)`; `Ok(false)` at clean
/// EOF before any prefix byte.
async fn read_frame_into<R>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    max_bytes: usize,
) -> Result<bool, FrameError>
where
    R: AsyncBufRead + Unpin,
{
    buf.clear();
    if buf.capacity() > RECLAIM_THRESHOLD {
        buf.shrink_to(INITIAL_CAPACITY.min(max_bytes));
    }
    let mut prefix = [0u8; FRAME_PREFIX_LEN];
    if !read_exact_buffered(reader, &mut prefix).await? {
        return Ok(false);
    }
    let declared = u32::from_be_bytes(prefix) as usize;
    if declared > max_bytes {
        discard_up_to(reader, declared).await?;
        return Err(FrameError::Oversized {
            declared,
            max: max_bytes,
        });
    }
    buf.resize(declared, 0);
    if !read_exact_buffered(reader, buf).await? {
        return Err(FrameError::Io(std::io::ErrorKind::UnexpectedEof.into()));
    }
    Ok(true)
}

#[cfg(test)]
#[path = "frame_tests.rs"]
mod frame_tests;
