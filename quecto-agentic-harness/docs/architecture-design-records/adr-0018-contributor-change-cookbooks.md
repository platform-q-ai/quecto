# ADR-0018 — Contributor Change Cookbooks for Common Harness Extensions

**Status:** Proposed.

**Implementation status:** Not started.

## Context

The harness has strong conventions: clean architecture boundaries, sibling test
modules, contract tests for ports, BDD scenarios for integration behaviour,
protocol compatibility rules, and documentation checks. These conventions help
maintain quality, but they are not obvious to new contributors or to agents
working in the repository.

Common changes often touch predictable slices of the system:

- adding a tool;
- adding a UDS command;
- adding a provider/model capability;
- adding a progress or audit event;
- changing session persistence;
- adding subagent behaviour;
- adding workflow capability;
- changing context pruning/recovery.

Without a documented change map, contributors discover required files and tests
by search and review feedback. That raises the cost of otherwise small changes
and increases architecture drift.

## Decision

Create contributor change cookbooks for common harness extensions.

Each cookbook should describe:

1. the architectural layer where the change starts;
2. the production files or modules usually involved;
3. required tests, including unit, contract, architecture, BDD, and repo-doc
   tests where applicable;
4. documentation updates;
5. compatibility concerns;
6. common pitfalls and examples.

Initial cookbook topics:

- Add a built-in tool;
- Add or change a UDS command;
- Add a provider/model runtime capability;
- Add a progress/audit event;
- Add session persistence fields safely;
- Add subagent lifecycle/forwarding behaviour;
- Add workflow tool/state behaviour;
- Change context pruning/spill/recovery policy.

The cookbooks are guidance, not a replacement for architecture tests. They
should link to ADRs, PRDs, and concrete examples in the codebase.

## Consequences

- New contributors and agents can make aligned changes faster.
- Reviewers can point to documented expectations instead of repeating them.
- The repo's architecture rules become teachable, not just enforced by failing
  tests.
- Documentation must be maintained as conventions evolve.
- Some cookbooks may initially be incomplete; they should prefer useful, concise
  guidance over exhaustive manuals.

## Alternatives considered

- **Rely on README and existing docs.** Rejected: the README explains the system,
  but not the per-change implementation path.
- **Encode everything as tests only.** Rejected: tests catch violations after the
  fact; cookbooks help contributors choose the right path before coding.
- **Create a large internal architecture manual.** Rejected: focused cookbooks
  are easier to keep current and easier for agents to retrieve on demand.
