# ADR-0011 — Stay JSON on the Wire

**Status:** Accepted.

**Supersedes:** ADR-0009 (binary payload encoding), ADR-0010 (BDD steps author
JSON; encode at the transport boundary).

**Implementation status:** No code change required — ratifies the current
JSON-on-the-wire state and retires two never-implemented proposals.

## Context

ADR-0009 proposed adopting a binary payload encoding (MessagePack/CBOR) for the
UDS protocol *if* two preconditions held: ADR-0008 shipped (so the JSON-shaped
subsystems that dominate the migration cost no longer exist), and profiling of a
real workload showed socket serialization/deserialization or frame size as a
material cost. ADR-0010 was its companion: keep BDD tests authoring JSON and
transcode at a shared test-transport boundary, so a later binary switch would be
a codec flag in the helpers rather than a rewrite of hundreds of step
assertions. Both have sat **Proposed / Not implemented**.

The situation has since changed decisively:

- **ADR-0008 is Accepted and shipped.** Its Part 2 (bounded end-of-turn events —
  events reference messages by stable id instead of re-carrying full content)
  landed via #1060 (merged 2026-07-13). End-of-turn payloads collapsed from
  whole-conversation blobs to small ref lists; `get_message` resolves refs on
  demand. The frame/size pressure that motivated a binary codec is gone.
- **The single measured precondition never triggered — and got less likely.**
  ADR-0009 itself records that serialization CPU and frame size have never been
  observed as bottlenecks; the #1047 incident was framing and content growth
  (ADR-0008's territory), not encoding. #1060 further shrinks the typical
  payload, so precondition 2 is now *further* from being met, not closer.
- **Binary's costs are real and paid today in debuggability.** JSON on the wire
  keeps `socat` + eyeballs inspection, keeps BDD steps asserting on decoded JSON
  fields against the real socket, and needs no migration (or transcoding bridge)
  for external extension processes that speak the protocol. The serialize-once
  `serde_json::RawValue` byte-budgeting in `build_get_messages_line` (#994) has
  no MessagePack/CBOR equivalent.
- **ADR-0010's payoff was contingent on ADR-0009.** Its headline benefit was
  "if binary is adopted, the tests are a codec flag, not a rewrite." With binary
  rejected, the wire format and the test *authoring* format stay the same
  (JSON), so there is nothing for a transcode boundary to absorb. The one
  encoding-independent good it bundled — replacing raw-line `line.contains(...)`
  substring checks with parsed-field assertions — stands on its own as ordinary
  test hygiene (already the repo's BDD-quality bar per the #1051 review and
  ADR-0007) and needs no transport-boundary machinery to justify it.

Carrying two speculative "Proposed" ADRs, a standing reminder to revisit them,
and a not-yet-built test-transport abstraction is ongoing design overhead for an
optimization with no measurement behind it. This is a YAGNI call.

## Decision

**JSON (UTF-8, `serde_json`) stays the UDS payload format.** No binary encoding
(MessagePack, CBOR, bincode, protobuf) is adopted, and the test-transport
transcode boundary from ADR-0010 is not built.

- **Production** keeps serializing protocol types directly to JSON, framed per
  ADR-0008 (length prefix). Disk formats (session files, audit log) were always
  JSONL and are unaffected.
- **Tests** keep authoring and asserting JSON values directly, as they do today,
  against the real socket via the existing `quecto-line-io` helpers. No
  JSON⇄codec transcode layer is introduced.
- **ADR-0010's test-design principles are retained; only its transcode boundary
  is dropped.** What survives on its own merits, independent of any encoding
  switch: scenarios pin protocol *behaviour* (which commands produce which
  events with which fields), not byte layout; steps assert on parsed values, not
  raw-line substrings (migrating residual `line.contains(...)` checks remains the
  standing BDD-quality bar per the #1051 review and ADR-0007); the suites drive
  the real socket end-to-end through `quecto-line-io`; and defence/boundary tests
  that deliberately build malformed or oversized input use shared
  frame-construction helpers (`build_frame` / `build_oversized_frame`) against
  the real format rather than hand-rolled byte literals. What is dropped is the
  JSON⇄wire *transcode* seam whose only distinct payoff was making a future
  binary switch a codec flag — with binary rejected, it absorbs nothing.
- **Reopening bar.** If a real, profiled workload ever shows socket
  serialization/deserialization or frame size as a material cost that ADR-0008's
  framing and #1060's refs do not relieve, that is grounds for a **new** ADR
  starting from those measurements — not a revival of ADR-0009's speculative
  framework. A self-describing serde codec swap post-ADR-0008 remains a
  contained, mechanical change should the evidence ever demand it; this ADR
  declines to pay for it, or to structure the tests around it, on spec.

## Consequences

- ADR-0009 and ADR-0010 are retired (`Rejected — superseded by ADR-0011`); the
  standing "revisit ADR-0010 after #1060" follow-up is closed.
- The wire stays human-readable: `socat` inspection and JSON-field BDD
  assertions keep working; extension processes need no encoding migration.
- No new test abstraction is built or maintained; the BDD suites keep driving
  the real socket path through `quecto-line-io` with JSON authoring.
- The repo carries one less speculative optimization framework. The door to
  binary is not welded shut — it is gated on measurement via a future ADR, so no
  analysis is lost (the format constraints and post-ADR-0008 cost picture live
  in ADR-0009's history).
- Cost: if serialization/size ever *does* become hot, the migration starts from
  a clean-slate ADR rather than a pre-negotiated plan. Accepted deliberately —
  the plan was never validated against a measurement, and #1060 makes the need
  less likely, not more.

## Alternatives considered

- **Keep ADR-0009/0010 as Proposed and periodically revisit.** Rejected: that is
  the overhead this ADR removes. The single measurable trigger has not fired in
  the time since, and #1060 moved it further away; indefinite "revisit later"
  status is worse than an explicit, reversible rejection.
- **Reject binary but still build ADR-0010's test-transport boundary.** Rejected:
  with no wire-encoding change coming, the boundary absorbs nothing — it is pure
  indirection over the framing `quecto-line-io` already owns. Its only durable
  benefit (parsed-field over substring assertions) is kept without it.
- **Edit ADR-0009/0010 in place to reflect the new decision.** Rejected: ADRs
  are immutable point-in-time records. The reasoning that led to them is history
  worth preserving; superseding with this ADR and flipping their status pointer
  keeps the trail intact.
- **Weld the door shut — rule out binary permanently.** Rejected for the same
  reason ADR-0009 rejected it: a self-describing codec swap stays a contained,
  mechanical option post-ADR-0008. The decision here is "not on spec, not now,"
  not "never" — the difference is a measurement.
