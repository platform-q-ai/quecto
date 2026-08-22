# ADR-0024 — Observation Tools Use Projection, Delta, or Snapshot-to-File Outputs

**Status:** Accepted.

**Implementation status:** Contract ADR for observation-tool output shaping. First implementation work is tracked by #1512–#1516, #1518–#1520, and #1522.

## Context

Session analysis of 10 parent orchestration sessions from Aug 9–14 measured about 1.12M observation-result tokens. The profile attributed 61–66% of parent context traffic to mechanical observation data: repeated full-state payloads returned by polling-style tools rather than new decision-relevant information (#1524).

The current harness has protocol-level protection but that is not an output contract. The UDS frame cap is 8 MiB: `PROTOCOL_LINE_CAP_BYTES` is `8 * 1_048_576`, and `PROTOCOL_FRAME_CAP_BYTES` deliberately reuses that value (`quecto-line-io/src/lib.rs`, `quecto-line-io/src/frame.rs`). The framed reader reports an oversized declaration and discards the declared payload so a following frame can still parse (`quecto-line-io/src/frame.rs`). Existing code comments also treat the shared 8 MiB cap as an emission bound, not as a shaping policy (`quecto-agentic-harness/src/application/agent_loop.rs`).

Observation tools such as `agent_cmd`, `workflow`, history/catalogue readers, and future orchestration/status tools therefore need a product-level contract for what they may return inline. Without one, each new tool can regress to returning complete operational state on every call, forcing parent sessions to spend context on unchanged snapshots and risking over-cap transport failures.

## Decision

Every observation tool must expose one or more of these output modes. The default mode must be bounded and safe for repeated calls.

### 1. Projection by default

A no-cursor observation returns a projection: bounded, decision-relevant fields only. The projection is the tool author's explicit answer to "what does a caller usually need to decide the next action?" It must not include bulk logs, full histories, static rosters, schemas, or unchanged nested state merely because those fields are available internally.

Projection responses must state their freshness with metadata such as generation, sequence, timestamp, observed-at time, or an equivalent domain cursor.

### 2. Delta via cursor

Observation tools with changing state must support cursor-style reads when callers need to poll. A request such as `since: <cursor>` returns only changes after that cursor plus the next cursor. If nothing changed, the response is a small unchanged marker carrying the current cursor/freshness metadata.

Cursor semantics are domain-specific, but they must be monotonic enough for callers to avoid re-reading the same state. Cursors are not a request for static metadata; static or echo data must stay out of operational delta responses.

### 3. Full fidelity to file

Complete snapshots, full histories, large logs, schemas, or diagnostic payloads go to a file path in the workspace/session artifact area, never inline. The inline response may summarize the snapshot, report byte/item counts, include freshness metadata, and name the path to read for full fidelity.

When an inline response would approach or exceed the protocol frame cap, the tool must degrade to truncation plus snapshot-to-file. It must never rely on the transport cap as normal control flow. In particular, the known 8 MiB hazard is that an over-cap framed response can be discarded/rejected by the transport layer and surface to callers as an unhelpful timeout or protocol error rather than a useful truncated observation. The mitigation is mandatory: shape first, spill full fidelity to a file, and return a bounded pointer.

## Static data exclusion

Static or echo data does not ride operational observation responses. Examples include enum lists, template rosters, tool schemas, descriptors already available from a catalogue, command echoes, and repeated invariant configuration. Such data belongs in documentation, catalogue/schema endpoints, or an explicit one-time metadata request. Operational responses may include stable identifiers that let callers join against that static data when needed.

## Consequences

- New and modified observation tools have a shared review checklist: default projection, cursor delta when polling is expected, snapshot-to-file for full fidelity, freshness metadata, no static-data echo, and over-cap degradation.
- The first conforming implementations are tracked in the existing per-operation issues #1512–#1516, #1518–#1520, and #1522.
- Callers should prefer projection or delta reads in orchestration loops and request snapshot files only when they genuinely need audit/debug fidelity.
- This ADR does not prescribe exact field names for every tool. Each tool may choose domain-appropriate names, but the semantics above are mandatory.
- The success target is a re-profiled aggregate result-token reduction of at least 60% versus the frozen Aug 9–14 baseline, while preserving full-fidelity recovery through files.
