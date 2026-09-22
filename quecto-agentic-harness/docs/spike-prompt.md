Goal: Spike #<N>: attempt a working end-to-end implementation quickly, without new tests, BDD/TDD or polish, while respecting the architecture. Expose unknowns that reading the code cannot reveal, and challenge the issue’s assumptions and assertions before sizing and planning. The code is disposable; later delivery is built properly, test-first.

Start from the truth: if #<N> has a Challenge report, its "What is actually left" paragraph and its traps are your brief — the issue body may be stale. If it has none, check the issue against the code for a few minutes first.

How to spike:
- Build the whole thing end to end, the crude way. Follow the compiler and the running behaviour, not a plan. Fix whatever breaks only as far as you need to keep going.
- Crude in polish, never in structure. Respect the project's architecture: Clean Architecture (code lives in the layer it belongs to, dependencies point inward, boundaries are crossed only through their ports) and SOLID. A spike built in the wrong place tells you nothing about where the real work lands or how it cuts. If doing it properly needs a new port, type or seam, create it roughly — that need IS a finding. If the architecture makes the change awkward, do not cut across it: record the friction.
- No new tests, no docs, no version bumps, no refactoring for taste. Hard-code values, stub data and skip edge cases freely — but write down every shortcut, because each one is work the real implementation still owes.
- When something resists — a design that does not work, a hidden caller, a file at its size limit, a test that pins the old behaviour, a decision only the owner can make — note it and route around it. Those are the findings.
- If an approach fails, say so and try another. The dead ends are as valuable as the one that worked.
- After implementation stops, run the existing test suites ONCE, unchanged — including the project's architecture checks — and record what fails, by name. If time or environment prevents a run, say which checks were not run and why.
- Implementation timebox: <TIMEBOX, e.g. 60 min>. Stop implementation when it ends, finished or not. The additional reporting window is only for running existing checks, pushing the branch, and reporting findings; do not continue building.

Keep it disposable: work on a branch named spike/<N>-<slug> in its own worktree; push it as a spike branch so the planner can read the diff; never open a PR, never merge, never cherry-pick from it. It is deleted, locally and on the remote, once the plan is filed.
Project rules: <PROJECT_RULES, or "none">. Environment problems you cannot fix: pause the swarm and raise the concern with the exact error.

Post ONE Spike report comment on #<N>:
1. Did it work? — one paragraph: what you built, how far it got, the branch name and `git diff --stat`.
2. What came up — the surprises, in order of how much they change the work.
3. The approach that worked, and the ones that did not — enough that an implementer starts from the right design, and why. Name each new or changed port, type and seam and the layer it lives in; say where the architecture resisted.
4. The map — list every changed file, grouped by WHY (core change · callers that had to follow). Separately name relevant tests pinning old behaviour, documentation needing later updates, and callers discovered but not changed. Do not modify tests or docs in the spike.
5. What broke — existing tests that fail, by name, and for each: asserts the old shape only, or something really changed. State explicitly if checks were not run and why.
6. Shortcuts taken — everything faked, skipped or hard-coded that the real implementation must do properly.
7. Where it wants to be cut — the places the diff falls apart into pieces that could each merge alone, and the order they had to happen in. Observations only; slicing is the Plan's job.
8. Questions only the owner can answer, each with what you assumed in order to keep going.

Criteria:
- really-built (review): an end-to-end attempt exists on the pushed spike branch; the report says honestly how far it got
- architecture-respected (review): the spike's code sits in the right layers with dependencies pointing inward; the project's architecture checks were run and any failure is reported as a finding, not worked around
- findings-not-polish (review): no new tests, docs or clean-up in the diff; shortcuts and dead ends are listed
- map-from-the-diff (command): section 4's changed-file list matches `git diff --name-only` against the spike branch base; unmodified tests, docs and callers are listed separately
- broke-by-name (review): failing tests from one real run are named and marked old-shape or real change; any checks not run are identified with reasons
- disposable (command): no PR, no merge, no cherry-pick; branch is spike/<N>-…
Members: choose a task-appropriate fixed pool (typically 1–3); coordinate only where parallel work speeds discovery. Deadline: the implementation timebox plus up to 60 min for checks, push and report.
