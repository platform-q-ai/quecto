# P1 RED evidence

Baseline contracts 77/77 and architecture 46/46 passed before new tests. P0 fixtures
4+2 passed. New public contracts command:
`cargo test -p quecto-agentic-harness --test contracts inference_admission --quiet`
failed before implementation with E0432 missing application::inference_admission,
domain::inference_admission and the three application port exports. Full local
output: /tmp/p1-red-compile.log. This establishes missing behavior/API, not yet
individual assertion falsifiability. Individual mutations will be recorded below
before publication; no checked-in failing tests are permitted.

Real behavioral RED after safety review: `cargo test -p quecto-agentic-harness
--test contracts admission_dispatcher --quiet` failed at assertion `eligible
background must receive a shared opportunity` with one long interactive active
transport, C=2/R=1, and four short interactive completions. Fixed reserve occupancy
allocation (interactive occupies reserve first; shared grants alone spend streak).
Identical targeted command passed 1/1. Logs /tmp/p1-reserve-{red,green}.log.

Registry boundary contract: three individual assert_eq→assert_ne inversions each
failed at intended assertion, logs /tmp/p1-registry-mutant-{1,2,3}.log. Original
restored. Tests unissued registration/enqueue and retired-parent registration.

Per-source assertion inversions prove executable expectations, not mutation score
against all plausible production faults; the real reserve-accounting mutation and
independent semantic reviews supplement this limited claim. Loop iterations are
covered by restored table execution but not each separately inverted.
Final semantic correction: short_interactive_attempts_cannot_starve_background_
pacing_opportunities actual RED at exact [I,I,I,B] assertion (observed [I,I,I,I]),
then corrected reserve-pacing accounting and full 99 contracts GREEN. Existing
reserved-grant shared-streak contract remains unchanged and GREEN.
