# Authoring issues / PRDs

Issues in this repo are behavioural PRDs: they describe the **outcome** and its
**boundary**, not the implementation. This exists because implementation-first,
boundary-less issues manufacture scope creep — an issue that named a specific
file and an illustrative command once produced a 383-line / 18-file PR for what
should have been a five-file change, plus a bogus conformance failure chasing a
command that never existed.

Use the `Feature / change (behavioural PRD)` issue template. The sections:

## Problem / Outcome
The user-facing problem or the behaviour we want — the *what*, never the *how*.

## Acceptance criteria (behavioural, checkable)
Observable outcomes anyone can verify, in domain language.
- **No implementation detail** — no file names, function names, or struct fields.
- **No phantom commands** — never write "e.g. run `foo`" unless `foo` already
  exists; an illustrative command reads as a spec and gets built.
- Each criterion should map to a scenario the workflow's `conformance` step can
  check against the branch code.

## Out of scope / Non-goals (REQUIRED)
The single most important section. State explicitly what the change does **not**
include — e.g. "No new CLI/UDS commands", "No provider-routing changes", "Does
not change defaults". This gives the implementer a hard boundary and gives the
reviewer/conformance step something to flag *scope creep* against, not just a
checklist to satisfy.

## Constraints
State the expected shape so an oversized change is visibly wrong (e.g. "registry
entry + specs + doc sync — a handful of files"). Self-contained, repo
conventions, YAGNI, no speculative abstraction.

## Facts & sources
Any external value (specs, versions, pricing) MUST cite a source URL. Confirm,
don't guess — wrong values silently corrupt behaviour (e.g. a model's output cap
feeds the token clamp).

## Hints (non-binding — NOT acceptance criteria)
Optional pointers to likely code areas, explicitly labelled non-binding. The
implementer may ignore them; they must never be treated as requirements.

## References
Related issues / PRs / links.

---

**How the workflow uses this:** the `scenarios` step turns the behavioural ACs
into feature scenarios; the `conformance` step verifies each AC against the
branch code **and** treats the Out-of-scope section as a boundary (work outside
it is a finding, not a gap). Keep the two in sync: behaviour and boundary.
