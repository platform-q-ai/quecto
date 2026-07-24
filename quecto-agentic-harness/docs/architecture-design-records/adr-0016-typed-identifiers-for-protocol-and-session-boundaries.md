# ADR-0016 — Typed Identifiers for Protocol and Session Boundaries

**Status:** Proposed.

**Implementation status:** Not started.

## Context

The harness moves many string identifiers across boundaries:

- session keys;
- agent/subagent ids;
- message ids;
- tool call ids;
- UDS command correlation ids;
- provider model identifiers;
- extension names;
- workflow template and step keys.

Many of these are plain `String` or `&str` values. That is convenient, but it
makes accidental mixing possible at the exact places where mistakes are most
costly: UDS command handling, subagent forwarding, history retrieval,
persistence reconciliation, and audit/progress correlation.

The risk grows with multi-agent sessions because parent ids, child ids, session
keys, command ids, and message ids are often present in the same functions.

## Decision

Introduce narrow typed identifiers at protocol and session boundaries.

The target pattern is lightweight newtypes with explicit construction and
serialization behaviour:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionKey(String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageId(String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ToolCallId(String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CommandId(String);
```

The exact module placement and validation rules may vary by identifier. The
initial goal is type safety and readability, not a large validation framework.
Identifiers should continue to serialize as strings to preserve protocol and
persistence compatibility.

Adoption should be incremental. Start where multiple id kinds coexist:

1. UDS command structs and response correlation;
2. subagent forwarding and child history resolution;
3. session persistence and message lookup;
4. audit/progress correlation paths.

## Consequences

- Function signatures become more self-documenting.
- The compiler can prevent common id-mixing mistakes.
- Serialization remains backward-compatible if newtypes are transparent or
  serialize as strings.
- Some tests and mocks will need small construction helpers.
- Overuse can add boilerplate; only identifiers that cross meaningful boundaries
  should become newtypes.

## Alternatives considered

- **Keep all ids as strings.** Rejected for high-risk boundaries; convenience is
  outweighed by ambiguity as subagent/session complexity grows.
- **Introduce a single generic `Id<T>` type.** Deferred: it can work, but simple
  domain-specific newtypes are easier to read and document.
- **Add strict validation for every id immediately.** Rejected: validation rules
  may differ by source and protocol generation. Type safety can land first.
