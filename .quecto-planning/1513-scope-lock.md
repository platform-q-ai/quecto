# #1513 scope lock

Phase: one feature slice implementing default `agent_cmd get_messages` report/delta semantics.

Acceptance criteria: all nine checkboxes in issue #1513 (first-contact latest assistant response; subsequent delta; unchanged marker; parent-registry persistence/restore; durable ordinal cursor; delivery-error non-advance; explicit history cursor-neutral; notification wording; embedded handoff docs; BDD coverage).

Non-goals: no mode or explicit cursor parameter; no changes to explicit `before`/`count` history semantics; no UUID cursor; no cohort re-profiling; no #1524 ADR; no broad #1525 schema slimming; no workflow/lifecycle redesign.

Expected surfaces: agent_cmd tool/dispatch; parent-scoped subagent registry persistence; UDS query/history/snapshot paths as required; agent_cmd BDD feature/steps and focused units; subagent/workflow docs and embeds.

Constraints: projection/delta/unchanged contract from #1524; mirror merged #1512's server-owned durable cursor and busy-path consistency; ~800-token bounded report with explicit truncation; advance only after successful parent-context delivery; full paging remains deliberate reread and cursor-neutral.

Verification: RED/GREEN BDD for all specified paths; focused unit/integration tests; docs embed verification; fmt; strict clippy; repository hooks/full push gate.

Deferred: post-merge cohort measurement; broader observation ADR; general schema footprint reduction.
