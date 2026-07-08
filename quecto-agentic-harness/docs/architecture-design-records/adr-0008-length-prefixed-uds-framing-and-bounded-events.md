# ADR-0008 — Length-Prefixed UDS Framing and Bounded Events

**Status:** Proposed.

**Implementation status:** Not implemented.

## Context

The TUI↔harness protocol (and the same protocol reused for sub-agent
monitoring and external extension processes) is newline-delimited JSON over a
Unix domain socket: one JSON document per line, framed only by the trailing
`\n` byte. This framing is entirely quecto-owned — providers never see it
(provider traffic is a separate HTTPS/SSE hop, normalized before it reaches
the socket; see ADR-0001 for the kernel-owned wire-protocol boundary).

Issue #1047 exposed the structural weakness. Because `\n` is the *only* frame
delimiter, a reader cannot know a message's size until the delimiter arrives,
so it must buffer blindly. The defensive answer was a 1 MiB per-line cap
(`quecto_line_io::PROTOCOL_LINE_CAP_BYTES`), and near a full context window
legitimate `turn_end`/`agent_end`/`get_messages` events exceeded it: the TUI
dropped the lines unread, sessions looked frozen, and the disconnect was
undiagnosable. PR #1051 fixed the incident by making the harness shrink
oversized events to fit under the cap (`infrastructure/line_cap.rs`: tailing
message arrays, preserving the newest content) and making the TUI count and
surface drops. That machinery works, but it is *active* in normal operation —
the transport regularly relies on lossy, content-aware truncation to function.

Two distinct problems hide in this:

1. **Framing blindness.** The reader discovers oversize mid-buffer and can
   only truncate or drop; there is no way to reject a frame cheaply, skip it,
   or negotiate.
2. **Unbounded event content.** Events redundantly carry entire conversation
   payloads: `turn_end`/`agent_end` re-ship full message content the TUI
   already received as streamed tokens; connect/resume ships history as one
   monolithic `get_messages` line. Frame size grows with conversation size by
   design, so *any* fixed transport bound will eventually be hit.

The context-management ladder (#1046: full → stub → recall-on-demand) already
solves the analogous problem for LLM context, but does not apply to what is
shipped over the wire to the UI.

## Decision

Evolve the socket protocol in two coordinated parts. Payloads remain JSON
(binary encoding is deliberately out of scope — see ADR-0009).

### 1. Length-prefixed framing replaces newline framing

Each frame is a fixed-size big-endian byte-length prefix followed by exactly
that many bytes of UTF-8 JSON. `quecto-line-io` (already the single framing
choke point on both sides since PR #1051) gains `write_frame`/`read_frame`;
the ~4 production read sites (TUI client, harness `uds_reader`, sub-agent
parent reader, extension protocol reader) and the centralized emit helpers
switch over.

The maximum frame size stays — a declared 10 GiB frame must still be
refusable — but its character changes: the reader learns the size *before*
buffering, so an oversized frame is rejected deliberately with a clean,
loggable protocol error instead of mid-buffer truncation heuristics.

**Compatibility.** The TUI, harness, sub-agents, and extension processes are
separately versioned binaries; framing is a hard wire break. The socket
announcement (`quecto-agent-socket: …` on stderr) grows a protocol-version
token, and readers sniff the first byte of a connection (`{` = legacy NDJSON,
otherwise a frame prefix / magic byte) for one deprecation window, after
which NDJSON support is removed.

### 2. Events become bounded by construction

- **End-of-turn events reference, not re-carry.** `turn_end`/`agent_end`
  ship message IDs plus deltas the client does not already hold, instead of
  full message payloads it received token-by-token during the turn.
- **History is paged.** Connect/resume snapshots return the newest page plus
  a cursor; the TUI backfills older history on demand (scroll-back) via
  offset/cursor variants of `get_messages_tail`. Snapshot size becomes
  independent of session size, and `data.trimmed` semantics are replaced by
  explicit pagination.
- **The wire inherits the demotion ladder.** Where the UI needs long content
  the ladder has stubbed, the wire ships the stub and the client recalls full
  content on demand — the same full → stub → recall model as #1046, applied
  to UI transport.

### 3. The size cap becomes an invariant, not a mechanism

With (2) in place, no legitimate event approaches the frame limit. The
shrink/tail machinery from PR #1051 (`line_cap.rs` shrink arms, capped-emit
tailing) is deleted rather than ported; an oversized frame is rejected with a
`tracing::warn!` and telemetry, because at that point it is evidence of a
bug. The drop-counting/surfacing UI from #1051 is retained as the user-facing
signal for that invariant firing.

## Consequences

- The #1047 failure class (legitimate traffic silently exceeding a transport
  bound) is eliminated structurally instead of patched content-aware.
- One hard compatibility break, managed by version negotiation and a
  deprecation window; sub-agent and extension-process readers migrate in the
  same change since they share `quecto-line-io`.
- Resume of very large sessions changes from "newest ~1 MiB slice, silently
  trimmed" to explicit paging with backfill — a UX improvement, but the TUI
  gains cursor/backfill state it does not have today.
- `socat`-style debuggability is mildly reduced (frames are not line-readable);
  payloads remain JSON, so a trivial tap/decode helper restores it.
- PR #1051's non-transport work (panel persistence, child exit diagnostics,
  stderr drain, sub-agent group cleanup) is unaffected and remains.

## Alternatives considered

- **Keep NDJSON, keep shrinking.** Rejected: the transport stays lossy under
  normal operation, and every new growth-prone event type needs a bespoke
  shrink arm (`get_messages` was missed exactly this way in #1047).
- **Raise the cap.** Rejected: any fixed bound loses to conversation growth;
  blind buffering and stall-risk get worse, not better.
- **Jump straight to binary encoding.** Deferred to ADR-0009: it does not fix
  framing blindness, and its main costs are in code this ADR deletes.
