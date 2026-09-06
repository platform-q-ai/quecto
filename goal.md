# Goal: empirically improve community workflow examples

Branch: `chore/workflow-quality`.

Improve the built-in investigate, chore, bugfix, feature, and refactor workflows to world-class, general-purpose software engineering quality. Refer back to this file throughout the session, particularly before each experiment/revision cycle and before declaring completion.

## Required loop

1. Run one low-effort agent per workflow, each in its own fresh container. Give each a representative task and ask it to select its assigned workflow for the baseline.
2. Observe actual execution: retain tool-call transcripts, workflow progression, diffs/artifacts, and verification results rather than relying on final claims.
3. Use an independent LLM judge/panel to score work quality, behavior, and workflow adherence with a fixed evidence-based 100-point rubric. Critical correctness or integrity failures prevent a qualifying result.
4. Ask panel members to propose and vote on improvements to the workflow template itself. Avoid compensating with benchmark-specific task instructions.
5. Revise the workflow, bind the revised template at spawn using workflow_spec, and repeat the same task with a new low-effort agent in a fresh container. Keep other conditions comparable.
6. Once a workflow first scores at least 90/100, update its source file on this chore branch. Continue revising and validating until each workflow has three consecutive fresh qualifying runs. Include varied tasks to check generality, not just repeated-task optimization.
7. Preserve evidence before cleaning up old containers; clean them up as work progresses.
8. Continue this loop until all five workflows meet the target. Report blockers honestly; do not invent scores, evidence, or completion.

## Engineering principles

- Workflows must be language-, framework-, platform-, hosting-provider-, and tooling-neutral, not Rust- or GitHub-specific.
- Prefer TDD where applicable: demonstrate a meaningful failing check before implementation and verify the resulting change. Use appropriate evidence-first alternatives for read-only investigation, documentation, configuration, or other tasks where executable tests are inappropriate.
- Evaluate substantive engineering behavior, not merely checked workflow boxes.
- Keep the rubric stable across revisions; preserve baseline and revised scores and panel votes.
- Keep changes purpose-aligned and preserve unrelated existing work.

## Deliverables

- Improved source workflow templates on this branch.
- Experiment tasks, versioned workflow candidates, panel feedback/votes, scores, and supporting evidence.
- A concise final summary of improvements, verification, limitations, and whether each workflow met the consistency target.
