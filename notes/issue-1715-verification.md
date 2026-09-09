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

- **Startup** (`admit_with`): a membership-free `_status` read (deadline of the
  run) answers whether a run was created; a workflow request into such a
  container is refused before joining, so no member row is left behind; an
  ordinary container keeps `--workflow`/guards/spec; the join then records
  participation and disables the workflow for swarm members.
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
|H1 startup read the run status through a `summary` RPC before joining, which the store refuses for a non-member, so every join into an existing container failed|(superseded in round 2) a membership-free `_status` read gates the workflow request before joining; the join records participation|`startup_workflow_follows_swarm_participation_not_containerization` joins as a member the store has never seen|
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

## Mutations

    W1 participates treats setup as swarm, W2 participates never true, W3 creation never refused,
    W4 spawn gate ignores participation, W6 startup ignores participation, W7 startup never
    disables workflow, W8 composition ignores the join answer, W10 create ignores an engaged
    workflow, W11 workflow tool never refuses, W12 join does not record participation,
    W13 supervisor tick does not record participation, W14 engaged probe ignores a selected
    template (killed by `workflow_engaged_reflects_guards_and_selected_templates_only`): killed.
    W5 (context conjunct in the spawn gate): equivalent, removed from the code.
    W9 (create does not flip participation immediately): masked, the supervisor tick records
    the same answer on its first observation; the explicit flip closes the window before that
    tick and stays.
    Round 2 re-run: W1 placeholder counts as swarm, W2 nothing counts, W3, W4, W6 startup refuses
    regardless of the run, W7 startup never refuses, W10, W11, W12 join does not record, W13 tick
    does not record, W14, W15 run_created ignores the deadline, W16 hooks never run: killed.
    W8 (post-join workflow disable) was equivalent to the pre-join refusal and is removed.

## Adversarial review 2 and fixes

|Finding|Fix|Proof|
|---|---|---|
|H1 join-before-gate left a live member row with a dead pid; the next reconcile failed the whole run|membership-free `_status` RPC (`SwarmContext::run_created`) gates the workflow request before any join; participation is recorded by the join|`startup_workflow_follows_swarm_participation_not_containerization` asserts no row for the refused member and a `running` status after `reconcile`; `run_created_reads_the_store_without_membership`|
|M1 `participates` keyed on status, so a bootstrap placeholder failed by a reconcile counted as a swarm|participation means a created run: the placeholder carries deadline 0, `create` requires a future deadline that survives every later status|`only_a_created_run_is_a_swarm`|
|M2 the selector nudge kept firing for members whose workflow was merely available when the run appeared|`Participation::on_participation` hooks run once on the transition; composition registers `set_selector_nudge(false)` on the engine; an already engaged engine is documented as not torn down|`participation_hooks_run_once_on_the_transition_and_never_revoke`|
|L1 note contradicted the code|note rewritten|—|
|L2 launch revalidation test never reached the launch path|drives `launch_uds_agent` with a config validated before the run existed|`swarm_worker_launch_revalidates_workflow_before_effects`|
|L3 `register_workflow_tool` test-only|cfg-gated with its re-export|—|
|I engaged probe sampled before `spawn_blocking`|read inside the create step|—|
