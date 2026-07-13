# ADR-0010 — BDD Steps Author JSON; Encoding Happens at the Transport Boundary

**Status:** Rejected — superseded by ADR-0011 (Stay JSON on the Wire).

**Implementation status:** Not implemented (and will not be — see ADR-0011).

## Context

The BDD suites (harness UDS steps, TUI client-defence and disconnect steps,
sub-agent monitor tests) exercise the socket protocol directly: steps build
command lines as JSON text, write them to a real socket, read event lines
back, and assert on parsed JSON fields. Today the *authoring format* (JSON in
step code and feature-file expectations) and the *wire format* (NDJSON) are
the same thing, so any wire change looks, at first glance, like a rewrite of
hundreds of step assertions.

ADR-0008 changes framing (length-prefixed frames) and ADR-0009 may later
change payload encoding (MessagePack/CBOR). Neither should force a semantic
rewrite of the test corpus: the scenarios pin *protocol behavior* (which
commands produce which events with which fields), not byte layouts.

Scope note: the JSON⇄wire transcode this ADR describes exists ONLY in test
helpers. Production code serializes protocol types directly to the wire
encoding, so ADR-0009's performance case is unaffected by keeping the tests
JSON-authored — the transcode cost lands on the test suite, where readability
is worth more than nanoseconds.

## Decision

Tests keep authoring and asserting **JSON values**; conversion to and from
the wire format happens in one shared test-transport helper layer, mirroring
production's `quecto-line-io` choke point.

- **One send/receive helper pair per suite.** Steps call
  `send_command(json)` / `recv_event() -> serde_json::Value` (the harness and
  TUI BDD suites already largely funnel socket I/O through shared helpers;
  the remainder migrate to them as part of ADR-0008). The helpers own
  framing and encoding: under NDJSON they append `\n`; under ADR-0008 they
  write a length prefix; under ADR-0009's binary encoding they transcode
  JSON→MessagePack on send and MessagePack→`Value` on receive.
- **Assertions stay on parsed values, not raw lines.** Steps that currently
  substring-match raw line text (e.g. `line.contains("agent_end")`) migrate
  to field assertions on the decoded `Value` — this is already the repo's
  BDD-quality bar (the #1051 review flagged substring checks standing in for
  parseability) and becomes mandatory once bytes are not text.
- **Feature files are untouched.** Gherkin scenarios describe commands,
  events, and field values; no feature text encodes framing or byte counts
  except via ubiquitous terms ("the event line cap" / "the frame size
  limit") whose numeric truth lives in the step definition next to the
  shared constant.
- **Boundary/defence tests get frame-construction helpers.** Tests that
  deliberately build malformed or oversized wire input (client-defence,
  bounded-read, cap-boundary tests) are the one place raw bytes are
  legitimate; they use explicit `build_frame(bytes)` / `build_oversized_frame
  (len)` helpers from the same test-transport layer so "malformed" is
  constructed against the real format, not a hand-rolled literal.

## Consequences

- ADR-0008's test migration cost collapses to the helper layer plus the
  residual raw-line assertions; the scenario corpus survives verbatim.
- If ADR-0009's binary encoding is adopted, the test suite is a codec flag in
  the helpers, not a rewrite — removing the largest remaining migration cost
  from ADR-0009.
- The helpers add one seam where tests could diverge from production
  framing; mitigated by implementing them ON `quecto-line-io` itself (tests
  use the production frame writer/reader, only the JSON⇄Value conveniences
  are test-side).
- Substring-matching steps get cleaned up ahead of need — a test-quality
  win independent of the protocol work.

## Alternatives considered

- **Rewrite steps in the wire format.** Rejected: couples hundreds of
  assertions to a byte layout, makes ADR-0009 prohibitively expensive again,
  and loses feature-file readability.
- **Golden byte fixtures.** Rejected as the primary mechanism: brittle under
  any schema evolution; acceptable only as targeted regression pins inside
  the defence tests.
- **A protocol simulation layer (fake transport).** Rejected: the suites'
  value is that they drive the real socket path end-to-end; a fake transport
  would stop pinning the production reader/writer.
