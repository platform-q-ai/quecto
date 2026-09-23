# ADR-0028: Advisory admission bindings for usable provider slots

**Status:** Accepted
**Date:** 2026-09-24
**Scope:** #2111; narrowly supersedes [ADR-0026](adr-0026-shared-inference-admission.md)'s fail-closed requirement for a usable provider slot without an effective binding.

## Context

A new built-in provider credential or `models.json` provider can make a provider
usable before its slot has been added to global `admission.bindings`. Refusing
agent and TUI startup in this case makes optional shared API-rate-limit
coordination an obstacle to using the provider. An absent binding is distinct
from a malformed admission policy or an unreachable required authority.

## Decision

When admission is enabled, compose every **usable** provider slot. Resolve its
exact binding first, then an applicable `"*"` or `"default"` fallback. A slot
with no effective binding remains selectable and its requests run **without
admission-broker gating**. Bound slots keep their gate for every actual attempt,
including retries and OAuth provider rebuilds. This exception supersedes only
the prior requirement that every constructed provider have a binding and the
associated fail-closed behavior on an absent binding; all other ADR-0026
admission lifecycle and policy decisions remain in force.

Emit a non-blocking, actionable startup warning naming each unbound usable slot
once per startup session, including slots from built-in credentials and usable
`models.json` entries (and configured compatible endpoints). Do not warn about
hypothetical built-ins without credentials or unusable registry entries. In a
mixed installation only unbound slots warn; if all usable slots are unbound,
the agent and TUI still start and warn; if all have effective bindings, including
through a deliberate fallback, no missing-binding warning is emitted. An
installation with no usable providers retains its existing no-provider error.
The warning must say that requests for those slots are not broker-gated and
recommend adding an alias/group binding when shared API-rate-limit management
is wanted. It must not imply that admission protects unbound traffic.

This is **not** an outage bypass: malformed admission sections, unknown aliases,
invalid group policies, unavailable required authority, disallowed live policy
changes and failures on bound attempts remain hard errors. An explicit binding
takes precedence over fallbacks and cannot be ignored because it is invalid.
Configuration changes do not hot-update a running broker: restart the broker
and agent/TUI processes to adopt policy changes; a binding in a file does not
prove that the broker's current policy includes it. See the
[operator procedure](../inference-admission.md#diagnosing-unbound-provider-slots).

## Consequences

Users can use newly configured providers immediately, at the cost of bypassing
shared pacing, queueing and cooldown for unbound requests; provider API rate
limits may therefore be exceeded. Operators who require broker management must
inspect actual slots and bind them explicitly or deliberately configure a
fallback. This advisory exception is not a security or admission guarantee.
