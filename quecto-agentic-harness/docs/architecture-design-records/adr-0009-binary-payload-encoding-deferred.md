# ADR-0009 — Binary Payload Encoding for the UDS Protocol (Deferred)

**Status:** Proposed — deferred with an explicit revisit trigger.

**Implementation status:** Not implemented.

## Context

With ADR-0008 (length-prefixed framing, bounded events) the socket protocol's
structural problems are addressed while payloads stay UTF-8 JSON. A natural
follow-on question is whether payloads should become binary (MessagePack,
CBOR, bincode, protobuf) for size and serialization speed. This ADR records
the analysis so the question is not re-litigated from scratch.

Facts that constrain the choice:

- **Format constraint.** The protocol types are serde types using internally
  tagged enums (`#[serde(tag = "type")]`). Self-describing formats
  (MessagePack via `rmp-serde`, CBOR via `ciborium`) support this directly;
  **bincode does not** (no `deserialize_any`), and **protobuf** would require
  redefining every protocol type as a schema plus codegen. "Binary" therefore
  realistically means MessagePack or CBOR unless a full protocol-type rewrite
  is separately justified.
- **The expensive part is JSON-*shaped* code, not the codec swap.** Four
  subsystems assume JSON text on the wire path: the cap/shrink machinery
  (`infrastructure/line_cap.rs`, `serde_json::Value` trees and JSON byte
  accounting), the serialize-once `RawValue` byte-budgeting in
  `build_get_messages_line` (#994 — `RawValue` has no MessagePack/CBOR
  equivalent; splicing pre-encoded elements into an array becomes bespoke
  binary construction), the pre-parse string sniffing on the dispatch hot
  path (`is_cancel_command`/abort/steer detection), and the sub-agent
  monitor's parse-modify-forward re-stamping.
- **ADR-0008 deletes most of that cost.** The shrink machinery and the
  monolithic snapshot path are removed or restructured by ADR-0008; after it
  lands, binary encoding shrinks to near the mechanical codec swap plus the
  hot-path sniff and re-stamp updates.
- **Boundaries.** Session files and the audit log are JSONL on disk and stay
  JSON regardless (same serde types, different sink). External extension
  processes speak the socket protocol; a payload-encoding change either
  migrates them or adds a transcoding bridge at that edge.
- **No measured need.** Serialization CPU and frame size have not been
  observed as bottlenecks; the #1047 incident was framing and content growth
  (ADR-0008's territory), not encoding.

## Decision

**Defer binary payload encoding.** Payloads remain JSON after ADR-0008.

Revisit only when **both** conditions hold:

1. ADR-0008 is implemented (so the JSON-shaped subsystems that dominate the
   migration cost no longer exist), **and**
2. profiling of a real workload shows socket serialization/deserialization or
   frame size as a material cost.

If revisited, the selected encoding is **MessagePack (`rmp-serde`) or CBOR
(`ciborium`)** — self-describing, serde-compatible with the existing tagged
types. bincode and protobuf are rejected for the reasons above. The migration
would reuse ADR-0008's protocol-version negotiation for the wire break, keep
disk formats JSON, and ship a tap/decode helper to preserve socket
debuggability.

## Consequences

- No churn now; the door stays open with the decision criteria written down.
- Anyone proposing binary encoding later starts from the format constraint,
  the four (post-ADR-0008: fewer) JSON-shaped subsystems, and a profiling bar
  to clear, instead of re-deriving the analysis.
- Debuggability (`socat` + eyeballs, BDD steps asserting on JSON fields)
  is preserved for as long as the deferral holds; test steps can encode
  JSON→binary at the transport boundary if the deferral is later lifted.

## Alternatives considered

- **Adopt MessagePack/CBOR now, alongside ADR-0008.** Rejected: it front-loads
  the migration into exactly the code ADR-0008 deletes, for a benefit nobody
  has measured a need for.
- **bincode / protobuf.** Rejected on format grounds (internally tagged serde
  enums; schema/codegen rewrite) independent of timing.
- **Never.** Rejected: if paged, bounded events still prove hot under
  profiling, a self-describing binary codec is a contained, mechanical change
  post-ADR-0008 — there is no reason to foreclose it.
