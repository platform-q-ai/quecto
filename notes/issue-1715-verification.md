# #1715 — workflow eligibility follows swarm participation, not containerization

## Root cause

Every container launch (`scripts/container-runtime/docker/{create,exec}.sh`)
sets the `QUECTO_SWARM_*` contract, so `SwarmContext::discover` succeeds for any
container agent and three gates treated "has a swarm context" as "is a swarm
agent": spawn validation refused `workflow`/`workflow_guards`/`workflow_spec`
for every non-local launch, CLI startup (`swarm_runtime::admit`) disabled the
workflow before joining, and runtime composition omitted the engine. The tool
descriptions and manuals stated the same blanket rule.

## Fix

The container's coordination store carries a placeholder `setup` run from its
bootstrap; a swarm exists only once a run has been created. Domain rule
`swarm::participates(status)` (`!= Setup`) is now the single gate:

- **Startup** (`admit_with`): reads the run status before joining; an ordinary
  container keeps `--workflow`/guards/spec; a join into a container whose run
  exists is refused before inference ("swarm admission rejected: workflow is
  unavailable for swarm agents").
- **Composition** (`build_tool_runtime`): the join answer decides whether the
  workflow engine, tool and guards are installed; one shared
  `Participation` handle per composition is injected into the spawn, swarm and
  workflow tools.
- **Spawn**: only a swarm participant's launches are swarm workers; host
  parents may launch or join containers with workflows.
- **Creation**: `swarm op=create` is refused while the creator is running a
  workflow (guards, bound spec or selected template:
  `validate_swarm_creation`); an idle available engine does not block it, and
  after creation the shared handle flips so the workflow tool refuses every
  action and local spawns reject workflows.
- Manuals, spawn/swarm tool descriptions and `docs/swarm.md` state the
  participation rule.

## Proof

Domain tests (`swarm_tests.rs`), spawn validation and launch revalidation
(`spawn_swarm_tests.rs`), startup admission over a real store before/after run
creation (`swarm_runtime_tests.rs`), runtime composition with and without a
created run (`tool_runtime_profile_tests.rs`), creation refusal and
participation flip (`swarm_control_tests.rs`), workflow tool refusal inside a
swarm (`workflow_tool_tests.rs`), BDD `swarm_coordination.feature` (worker
rejection, ordinary container keeps eligibility, workflow-engaged creator
refused); the supervision fixture (a coordinator with default workflow
availability creating a run) stays green.

## Adversarial review 1 and fixes

|Finding|Fix|Proof|
|---|---|---|
|H1 startup read the run status through a `summary` RPC before joining, which the store refuses for a non-member, so every join into an existing container failed|the join itself answers the status; workflow is gated on that answer after joining (a refused member exits and is reconciled like any startup failure); the extra RPC and `run_status` are gone|`startup_workflow_follows_swarm_participation_not_containerization` joins as a member the store has never seen|
|H2 participation was recorded only at composition and by the creator, so a member that joined before the run was created kept a live workflow|one `Participation` handle per process (created with the agent flags, passed to the join and the tool composition); every supervisor tick records participation from the run status, so the workflow tool refuses and launches reject workflows for every member once the run exists. An engine a pre-create member already engaged is not torn down; documented, with "create before spawning"|`a_supervisor_tick_records_participation_from_the_run`; profile test asserts the composition's handle|
|H3 process-wide workflow-engine `OnceLock` made lib tests order-dependent|per-composition `WorkflowEngineSlot` created in `build_tool_runtime`, injected into the swarm tool|`swarm_bridge::workflow_engaged(&slot)`|
|M1 docs promised exclusion the code only gave the creator|H2 fix; docs state the pre-create limitation|—|
|L1 three interpreter spawns at startup|the pre-join RPC is gone|—|
|L2 `control()` was test-only production code|gated `#[cfg(any(test, feature = "test-support"))]`|—|
|L3 builder parked in `swarm_control.rs` for the line cap|builders back on the type; result helpers split into `swarm_output.rs`|quality gate|
|L4 terminal-run join wording|the join's own admission error now comes first|—|

A store-level "create is refused once other members exist" rule was tried and
reverted: `startup_admission_counts_members_before_run_creation` pins the
existing contract that early members count toward the limit.
