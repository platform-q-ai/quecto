# Issue #2045 cross-folder notice inventory

This inventory records the documentation and protocol surfaces affected by the
cross-folder session notice change. It is intentionally limited to source-of-
truth locations; implementation details belong in the architecture map and
protocol reference.

| Surface | Source of truth | Required state |
| --- | --- | --- |
| Session lifecycle and startup wording | `quecto-agentic-harness/docs/sessions.md` | Describe a plain refusal/notice for a session from another folder; do not promise an interactive resume action. |
| UDS request/response contract | `quecto-agentic-harness/docs/uds-protocol.md` | Document typed refusal data and compatibility handling for action-bearing clients. |
| Harness architecture | `quecto-agentic-harness/docs/architecture/harness-architecture-map.md` | Keep ownership and boundaries aligned with typed notices; remove retired resume-action machinery from the inventory. |
| TUI presentation | `quecto-agentic-harness/quecto-agentic-harness/README.md` | Present bounded, sanitized plain notices; no decision/action picker for this refusal. |

## Retired terminology

The following are not valid cross-folder behavior and must not be added to
new documentation: an offered `open_original`/`fork_current` decision,
resume-action execution, or an action picker presented for a cross-folder
refusal. Existing historical references should be removed or explicitly
labelled as retired when the corresponding implementation is removed.

## Safety invariants

- Notices are bounded and sanitize control/invisible characters before display.
- Refusal data is typed and stable; malformed input remains an uncorrelated
  parse error rather than a partially interpreted command.
- Rendering a notice performs no session claim, save, child settlement, or
  process spawn.
