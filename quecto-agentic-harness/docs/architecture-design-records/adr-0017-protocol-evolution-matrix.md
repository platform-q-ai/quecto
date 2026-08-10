# ADR-0017 — Protocol Evolution Is Tracked by a Capability Matrix

**Status:** Proposed.

**Implementation status:** Not started.

## Context

The harness UDS protocol is an active compatibility surface. It is consumed by
`quecto-tui`, `quecto-api`, subagents, extension processes, tests, and ad hoc
operators using JSON inspection tools.

Recent and ongoing work includes length-prefixed framing, bounded events,
message references, `get_message`, paged history, oversized-frame handling,
legacy line compatibility, subagent forwarding, and JSON-on-the-wire decisions.
Those decisions are captured in ADRs and tests, but contributors still need a
single place to understand which protocol capabilities exist, when they were
introduced, which clients depend on them, and what legacy behaviour remains.

Without that map, protocol compatibility becomes tribal knowledge.

## Decision

Maintain a protocol capability matrix as the human-facing index for UDS protocol
evolution.

The matrix should live in harness documentation near `uds-protocol.md` and link
to relevant ADRs, issues, and tests. It should track at least:

- capability name;
- introduced version/ADR/issue if known;
- current status: proposed, shipped, deprecated, removed;
- client surfaces that depend on it;
- compatibility/legacy behaviour;
- primary tests or scenarios that pin it;
- notes on frame-size or recovery implications.

Example rows:

```text
Capability                  Status   Depends on      Legacy behaviour
length-prefixed frames       shipped  ADR-0008        legacy NDJSON accepted
JSON payloads                shipped  ADR-0011        binary rejected until measured
message refs in agent_end    shipped  ADR-0008/#1060  legacy full messages absent/empty
paged get_messages           open     #1061           count tail supported
ranged get_message           shipped  #1060/#1093     large content walks offset/nextOffset
subagent get_message forward shipped  #1060           parent history must not answer
```

The matrix is documentation, not a separate source of truth for code generation.
Protocol tests remain executable truth. The matrix must be updated whenever a
protocol-affecting ADR or PR lands.

## Consequences

- New contributors can understand protocol compatibility without reading every
  ADR and test first.
- PR review gains a checklist: if a protocol capability changes, update the
  matrix and tests.
- The TUI/API/subagent compatibility story becomes easier to audit.
- Documentation can drift; repo-doc tests should check links and presence, while
  reviewers enforce semantic updates.

## Alternatives considered

- **Rely only on ADRs.** Rejected: ADRs are point-in-time decisions. A matrix is
  better for current operational compatibility.
- **Generate the matrix from protocol types.** Rejected: type definitions do not
  capture legacy behaviour, client dependencies, or rollout status.
- **Put all protocol history into `uds-protocol.md`.** Rejected: the protocol doc
  should remain a user/client reference. The matrix is an evolution/change map.
