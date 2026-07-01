---
name: Feature / change (behavioural PRD)
about: Outcome-first, behaviourally-led request with an explicit scope boundary — no implementation detail.
title: ""
labels: []
---

<!--
Write this behaviourally: describe the OUTCOME, not the solution. Do NOT prescribe files,
functions, or specific commands — those belong (if at all) under "Hints" as non-binding.
The single most important section is "Out of scope": it stops scope creep and gives the
review/conformance step a boundary to check, not just a checklist to satisfy.
-->

## Problem / Outcome
<!-- The user-facing problem, or the behaviour we want. Outcome, not mechanism. -->

## Acceptance criteria (behavioural, checkable)
<!-- Observable outcomes anyone can verify, in domain language. NO file/function names and
     NO commands that don't already exist. Each should map to a scenario the conformance
     step can check. Avoid "e.g. run <command>" — an illustrative command reads as a spec. -->
1.
2.

## Out of scope / Non-goals
<!-- REQUIRED. Explicit boundary. What this change does NOT include.
     e.g. "No new CLI/UDS commands." · "No provider-routing changes." · "Does not change defaults." -->
-

## Constraints
<!-- Keep it tight: state the expected shape so an oversized change is visibly wrong. -->
- Expected shape: <e.g. "registry entry + specs + doc sync — a handful of files">.
- Self-contained; follow repo conventions; no speculative abstraction; YAGNI.

## Facts & sources
<!-- Any external value (specs, versions, pricing) MUST cite a source URL. Confirm, don't guess. -->
-

## Hints (non-binding — NOT acceptance criteria)
<!-- Optional pointers to likely code areas. The implementer may ignore these. -->
-

## References
<!-- Related issues / PRs / links. -->
-
