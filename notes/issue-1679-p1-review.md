# Semantic review resolution

Reviewer dace5b58 found lifecycle fencing and feedback boundary gaps. Accepted:
- Unknown completion is distinct from a fenced historical duplicate: quarantine
  all configured groups when a future unknown request cannot identify its group,
  retain occupancy, prohibit new grants.
- Operations bind epoch/scope/sequence; no caller-selected alternate grant ID exists
  in the in-memory port. Scope inputs must be bound to authenticated callers by P3; the P1 port trusts
  its caller and does not enforce caller-to-scope authentication. Test mismatched
  scope lookup and epoch unchanged state (not caller impersonation).
- Typed cooldown zero/max/max+1/overflow cases distinguish valid max merging from
  excessive/unrepresentable feedback making only that group unavailable.
- Feedback is part of confirmed completion exactly once, not a separate replayable
  mutation; terminal duplicates cannot extend cooldown or reset counters.

These are additions to the matrix, not deferred provider header parsing or IPC.
Fresh verifier dfcf1682-76c0-48cf-9dfa-4a943b1b44a4 reconstructed canonical P1
and returned no remaining medium/high design gaps. Matrix frozen for test design.

Test review 0f7a2473 accepted corrections: added reserve/shared interaction trace,
pre-deadline cancellation, active-retire refusal/queued-retire cancellation, live
cooldown max merge/exact boundary, separate accepted-duration overflow and dispatch
overflow, regressing completion/enqueue atomicity, unseen lower sequence replay,
active/terminal conflicts, quarantine dispatch denial, saturated-group isolation,
and evicted completion while replacement active. Duplicate map keys belong at a
future duplicate-preserving config parser (BTreeMap cannot contain duplicates).
Typed delay normalization/fallback base/jitter stays P2 per its explicit checklist;
P1 consumes validated durations and max-bound validation only. Application scoped
handles are trusted inputs; actual capability authentication remains P3.

BDD review a03a25d7 accepted: queue/cancel outcomes now observed explicitly,
dispatch moved into When and recorded for Then, pacing interval explicit in Given,
transport claim renamed confirmed attempt completion. Combined exact trace asserts
both pacing boundaries.
Falsifiability review's historical contract findings overlap coverage corrections.
Decline mandatory split of every lifecycle trace: these are ordered protocol traces
whose later assertions depend on earlier transitions; individual mutation runs
prove each assertion independently reached. Naming invalid config cases will be
improved after mutation work restores the shared test file. Duplicate-map scope
correction recorded above.
Safety finder 00b74500 demonstrated background denial behind a long interactive
with available total capacity. Verifier 8b182544 initially refuted as a mandatory
ADR violation (shared-first allocation could pin a shared stream indefinitely).
Parent adopts reserve-first interactive allocation as an explicit work-conserving
policy refinement: B<=C-R still enforced; reserved interactive grants don't advance
streak. No promise of bounded wall latency. Added real failing regression and fixed.
The old algorithm also failed to track fixed slot assignments across completions;
reserve-first occupancy removes ambiguous reassignment and unnecessary stranding.
Reserve verifier follow-up CONFIRMED original algorithm bug: shared A completes
while reserved B remains; original algorithm implicitly reclassifies B as shared
and loses newly available shared opportunities. Initial refutation withdrawn.
Reserve-first accounting is consistent with ADR and eliminates this ambiguity.
Lifecycle/memory/time reviewer b068d629 found no concrete in-scope bugs after the
reserve correction (read-only review of actual policy/application code).
Final phase conformance reviewer 58e7638a found no missing P1 checklist item but
identified two documentation overclaims. Corrected matrix/review to say unknown
future completion quarantines all groups and scope authentication is deferred P3;
the typed P1 port trusts its caller. No claim of cross-scope security enforcement.
Remaining test-style resolution: combined protocol trace retained intentionally;
every source assertion independently inverted/restored so preceding checks do not
mask falsifiability. Invalid constructor cases will include config Debug output,
which identifies the exact failing numeric/mapping boundary without fixture labels.
Final mandatory complete-diff architecture reviewer 7ddf3940: clean; all five
required diffs inspected (working tree/master range empty; staged complete), no
layering/reuse/removed-behavior/cross-file regressions. No runtime wiring.
Final semantic reviewer e956bda9 found short reserve-only interactive attempts can
monopolize pacing forever. Reproduced actual RED exact [I,I,I,B] trace; corrected
with separate contested reserve-pacing streak reset by shared grants, preserving
original reserve/shared trace. Full 99 contracts green. Matrix updated.
Both mandatory reviewers re-inspected all five complete diff forms after fix:
e956bda9 semantic clean, independently reran 22 admission contracts and original
reproduction [I,I,I,B]; 7ddf3940 architecture/scope clean. No unresolved findings.
Coverage follow-up reviewed complete diffs by both e956bda9 and7ddf3940: clean;
shared cfg(test) contract registration changes no behavior/gates. Semantic reviewer
independently reran22 lib admission tests. Conformance remains P1 PASS, coverage
percentage pending authoritative rerun.
