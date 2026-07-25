# Architecture Design Records (ADR) — Index

Point-in-time records of significant architecture decisions. **ADRs are
immutable**: once written, the decision body is not rewritten. When a decision
changes, add a **new** ADR that supersedes the old one and flip the old record's
`Status` to `Rejected — superseded by ADR-00NN` (the status line is the only
edit a superseded ADR receives — its reasoning stays intact as history).

**Status vocabulary:** `Accepted` · `Proposed` (may be deferred) ·
`Rejected — superseded by ADR-00NN`.
**Numbering** is monotonic and may skip (0004 was never used). Next free: 0020.

| ADR | Title | Status | Decides |
|----:|-------|--------|---------|
| [0001](adr-0001-wire-protocols-stay-kernel-owned.md) | Wire Protocols Stay Kernel-Owned | ✅ Accepted | Provider wire protocols are kernel-owned, not community-authored or generically pluggable. |
| [0002](adr-0002-reload-trigger-for-startup-loaded-surfaces.md) | Reload Trigger for Startup-Loaded Surfaces | ✅ Accepted | Startup-loaded surfaces refresh via an explicit reload trigger, not on-consume or a filesystem watcher. |
| [0003](adr-0003-uds-register-provider-for-dynamic-model-provider-registration.md) | UDS `register_provider` for Dynamic Model/Provider Registration | 🕒 Proposed (deferred; not in v1) | Reserves a `register_provider` UDS command for dynamic provider registration; deferred past v1. |
| [0005](adr-0005-knowledge-as-a-retrieval-surface.md) | Knowledge as a Retrieval Surface | ✅ Accepted | Knowledge is an on-demand retrieval surface, not a kernel index or system-prompt injection. |
| [0006](adr-0006-composable-unit-contract-is-kernel-orchestration-is-an-external-tool.md) | Composable Unit Contract Is Kernel; Orchestration Is an External Tool | ✅ Accepted | The kernel owns the composable-unit contract; orchestration (taskgraph/DAG) lives in an external tool, not the kernel. |
| [0007](adr-0007-review-finder-waves-adversarial-verification.md) | Review Finder Waves with Adversarial Verification and a Per-Assertion RED Gate | ✅ Accepted | Code review runs finder waves + adversarial verification; tests must pass a per-assertion RED gate. |
| [0008](adr-0008-length-prefixed-uds-framing-and-bounded-events.md) | Length-Prefixed UDS Framing and Bounded Events | ✅ Accepted | Multi-part UDS protocol overhaul: length-prefixed frames + bounded events (see the ADR-0008 series below). |
| [0009](adr-0009-binary-payload-encoding.md) | Binary Payload Encoding for the UDS Protocol | ❌ Rejected → [0011](adr-0011-stay-json-on-the-wire.md) | Proposed MessagePack/CBOR payloads under two preconditions. Rejected: measurement trigger never fired; JSON stays. |
| [0010](adr-0010-bdd-steps-author-json-encode-at-the-transport-boundary.md) | BDD Steps Author JSON; Encoding Happens at the Transport Boundary | ❌ Rejected → [0011](adr-0011-stay-json-on-the-wire.md) | Proposed a test-transport transcode boundary to insulate tests from a binary switch. Rejected with 0009; its test-design principles are retained by 0011. |
| [0011](adr-0011-stay-json-on-the-wire.md) | Stay JSON on the Wire | ✅ Accepted (supersedes 0009, 0010) | JSON stays the UDS payload format; no binary encoding and no transcode boundary. Binary reopens only via a future measurement-driven ADR. |
| [0012](adr-0012-explicit-agent-turn-state-machine.md) | Explicit Agent Turn State Machine | 🕒 Proposed | Agent turn execution becomes an explicit internal application state machine. |
| [0013](adr-0013-uds-command-family-router.md) | UDS Command Dispatch Uses Command-Family Routers | 🕒 Proposed | UDS command handling is split by command family behind the existing protocol. |
| [0014](adr-0014-context-management-is-a-first-class-application-subsystem.md) | Context Management Is a First-Class Application Subsystem | 🕒 Proposed | Context budget/pruning/spill/gauge/reconciliation logic gets a named application boundary. |
| [0015](adr-0015-subagent-lifecycle-state-machine.md) | Subagent Lifecycle Is an Explicit State Machine | 🕒 Proposed | Subagent process/run/notification lifecycle transitions become explicit and testable. |
| [0016](adr-0016-typed-identifiers-for-protocol-and-session-boundaries.md) | Typed Identifiers for Protocol and Session Boundaries | 🕒 Proposed | High-risk ids become string-serializing newtypes at protocol/session boundaries. |
| [0017](adr-0017-protocol-evolution-matrix.md) | Protocol Evolution Is Tracked by a Capability Matrix | 🕒 Proposed | UDS protocol capabilities are tracked in a [compatibility/evolution matrix](../protocol-capability-matrix.md). |
| [0018](adr-0018-contributor-change-cookbooks.md) | Contributor Change Cookbooks for Common Harness Extensions | 🕒 Proposed | Common harness extension paths are documented as focused cookbooks. |
| [0019](adr-0019-role-segregated-domain-ports.md) | Domain Ports Are Segregated by Role When They Grow | 🕒 Proposed | Broad ports split into role-focused traits when caller pressure justifies it. |

## The ADR-0008 protocol series

ADR-0008 is delivered in parts, each tracked by an issue:

| Part | Scope | Issue | State |
|-----:|-------|-------|-------|
| 1 | Length-prefixed framing (version negotiation) | #1059 | Done |
| 2 | Bounded end-of-turn events — reference messages by stable id | #1060 (PR #1092) | Done — merged 2026-07-13 |
| 3 | Paged history on connect/resume + TUI backfill | #1061 | Open |
| 4 | Frame-size cap becomes a should-never-fire invariant; delete shrink/tail machinery | #1062 | Open (blocked on #1060, #1061; co-requisite #1094) |

Related `get_message` resolution-completeness work (makes the cap a true
should-never-fire invariant): **#1094** (chunked/paged transfer for a single
message larger than the frame cap) and **#1093** (recall full content from the
spill store for collapsed refs, both resolvers). Per ADR-0011, all of this stays
JSON on the wire.
