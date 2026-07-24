# ADR-0019 — Domain Ports Are Segregated by Role When They Grow

**Status:** Proposed.

**Implementation status:** Not started.

## Context

The harness uses domain traits as ports for application logic. This is a good
fit for the existing clean architecture boundary: application code depends on
vocabulary and behaviour contracts, while infrastructure supplies concrete
adapters.

Some ports naturally grow as the product gains features. For example, a tool
registry may need to expose tool definitions, execute tools, track extension
names, update session-aware tools, and manage extension tool lifecycle. These
are related, but they are not always needed by the same caller.

Large ports make tests and mocks heavier, obscure caller intent, and encourage
call sites to depend on more capability than they need.

## Decision

When a domain port grows multiple independent roles, split it into role-focused
ports while preserving ergonomic composition at construction boundaries.

A broad port should be evaluated for segregation when:

- callers routinely need only one subset of methods;
- mocks implement many unused methods;
- extension/lifecycle methods sit beside hot-path execution methods;
- a new method would force unrelated implementations to care about unrelated
  concepts;
- naming the smaller role would make architecture clearer.

Target examples:

```rust
trait ToolCatalog {
    fn definitions(&self) -> &[ToolDefinition];
    fn tool_count(&self) -> usize;
}

trait ToolExecutor {
    fn execute(...);
}

trait ExtensionToolRegistry {
    fn extension_names(&self) -> Vec<String>;
    fn register_extension(...);
    fn unregister_extension(...);
}

trait SessionAwareTools {
    fn set_session_key(&self, session_key: &SessionKey);
}
```

This ADR does not mandate immediate splitting of every existing trait. It sets a
rule for future refactors: split ports when role boundaries are real, not just
for aesthetic interface minimalism.

## Consequences

- Call sites become clearer about which capability they require.
- Unit tests and mocks can be narrower.
- Infrastructure adapters may implement several small traits instead of one
  large trait.
- Composition roots may need adapter structs or trait-object bundles to keep
  construction ergonomic.
- Over-splitting is possible; role segregation should follow observed pressure.

## Alternatives considered

- **Keep one port per domain concept regardless of size.** Rejected: convenient
  early, but large ports obscure dependencies and burden tests.
- **Split every method into tiny traits immediately.** Rejected: noisy and not
  justified. Segregate only when roles are meaningful.
- **Move extension lifecycle out of domain entirely.** Rejected as a blanket
  rule: the application still needs a port for extension lifecycle even if the
  implementation is infrastructure-owned.
