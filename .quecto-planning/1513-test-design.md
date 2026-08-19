# #1513 test design (reviewed RED input)

## Independently falsifiable BDD scenarios

1. **AC1 first contact selects a finalized answer** — Given an unread transcript with an older assistant answer followed by tool/user messages and one latest finalized assistant answer, when plain `get_messages` is delivered, then only that latest assistant answer is returned in the exact changed envelope.
2. **AC1/2 no finalized answer** — Given first contact with only user/tool/in-progress assistant entries, when plain retrieval occurs, then exact `{"unchanged":true}` is observed and no cursor is consumed.
3. **AC2 unchanged after delivery** — Given a first report was delivered, when plain retrieval repeats without appended messages, then exact unchanged is returned.
4. **AC1 delta** — Given a delivered report and newly appended mixed-role messages, when plain retrieval occurs, then only entries with newer durable ordinals are returned chronologically.
5. **AC1 single-message boundary** — exactly-at-cap remains untruncated; one UTF-8 message over cap returns a safe summarized envelope with length/truncation metadata, commits that fully represented ordinal, then unchanged.
6. **AC1 omitted tail** — a multi-message delta over total cap returns only fully represented leading ordinals with `truncated:true`; after delivery the next plain call returns omitted later ordinals.
7. **AC3/4 restore/reload** — after report delivery and parent roster persistence/restore, reload the child's same transcript with regenerated message UUIDs and one appended durable ordinal; plain retrieval returns only the appended item.
8. **AC5 failed delivery sequence** — prepare report → inject failure before parent conversation/run-ledger append → retry returns identical ordinal/content → successful append/ack → following call is unchanged.
9. **AC3/5 persistence transaction** — inject save failure around delivered tool result/cursor persistence; restore cannot consume a report absent from restored parent context, and retry returns any not-durably-acknowledged report.
10. **AC6 count neutrality** — default delivered → explicit `count` reread → default remains unchanged.
11. **AC6 before neutrality with pending delta** — leave unread delta → explicit `before`/`count` page that displays it → next default still returns that unread delta.
12. **AC6 legacy paging** — valid `count`, `before`, and both preserve existing page shape/order/cursor/errors.
13. **Routed descendant** — top parent observes a descendant through its ancestor; the ancestor's own later plain retrieval still sees its unread child report; only top-parent post-delivery state advances.
14. **Busy snapshot** — an in-progress connect snapshot is history-only/cursor-neutral; after finalization, plain report retrieval returns the finalized assistant answer.

Scenarios use observable Given/When/Then outcomes and avoid field-order assertions.

## Focused unit/integration checks

- Exact changed fields `{messages,truncated}` and unchanged field `{unchanged}`; latest-assistant/no-assistant selection; durable ordinal ordering.
- Report shaper exact 3.2 KiB boundary, UTF-8 cut, single-message summary, omitted-tail ownership.
- Registry acknowledgment monotonicity, duplicate acknowledgment, UUID/display-label lineage isolation, ambiguous alias and wildcard rejection. These support—not replace—BDD behavior.
- Domain delivery hook executes only after both parent conversation and run-ledger append; injected failure proves the BDD retry sequence.
- Persisted roster backward-compatible serde default, cursor snapshot/restore, and failed-save restore behavior.
- Direct, routed descendant, and persisted-historical adapters produce equivalent observable report semantics; existing busy snapshot remains TUI/connect history.

## Paging argument decision table

| Input | Expected behavior | Cursor effect |
|---|---|---|
| `count:-1`, overflow, fraction, string/object | addressable argument error | none |
| non-string non-null `before` | addressable argument error | none |
| both absent or null | report mode | commit only after delivery |
| `count:0` | legacy explicit empty history page | none |
| `before:""` or unknown id | legacy history cursor error | none |
| valid `count` / `before` / both | legacy history page | none |

## Docs/schema/notification checks

- AC7: completion notification, spawn definition, agent_cmd definition, embedded docs, README/reference/capability matrix and stale repository-doc guards are atomically migrated from `(count 1-5)` / omitted-newest-page advice to plain report/delta advice. Audit all occurrences and internal no-arg history consumers, including TUI attach/resume/backfill, which must use explicit history or dedicated snapshots.
- AC8: source docs plus subagent/workflow embeds contain concepts (not brittle exact prose): bounded structured handoff with summary, checks, artifacts, blockers, recommended next action; plain delta and unchanged behavior; no new wire protocol claim.
- Tool schema says omitted/null args select report mode and explicit `count`/`before` are cursor-neutral history.
- Run repository docs/embed verification and protocol capability checks.

## Traceability

All nine issue checkboxes and every semantic-matrix high-risk row map above. Cohort re-profiling remains the documented post-merge success-measure follow-up.
