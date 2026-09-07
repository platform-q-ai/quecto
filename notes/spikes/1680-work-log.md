# #1680 exploratory implementation log

Live templates inspected: feature requires a canonical execution plan; investigate and plan forbid implementation; chore excludes behaviour changes. None fits this explicitly timeboxed, unplanned prototype. Use an isolated spike branch, narrow RED/GREEN tests, review and draft-only handoff, rather than inventing a production plan.

Scope hypothesis: opt-in container workbench repurposes Python Lab at registration (no parallel tool), embeds a Python SQLite helper under `python3 -I`, and offers a fixed-pool runner and deterministic fake-worker observation. Production migration/default registration remains deferred. #1679 has no dependency here: provider admission remains existing behaviour.

Validation observed:
- Embedded helper test RED: ModuleNotFoundError under Python -I; GREEN after compiled helper bootstrap.
- Python helper 11 tests pass; SQLite concurrent claim/dependency/token/terminal cases.
- Rust swarm tests 2 pass; Python Lab regression 55 pass; native registration 21 pass.
- cargo build --bins and clippy -D warnings pass; formatting clean.
- Real CLI processes against loopback fake provider: 3-slot pool (scripted coordinator + 2 harness workers), 5 tasks across both workers, peer messages, exact 1/4/9/16/25 evidence, coordinator completed; no paid inference.
- Automated actual-process integration passes cap/nested/overlap rejection and SIGINT DB cancellation. Reviewer found temp-dir lock bypass/timer placement/group-ID/settlement gaps; fixed canonical /tmp lock, timer before setup, capture process group IDs, bound settlement before killing/reaping.
- Full issue acceptance suite and paid-provider behaviour are NOT validated. Review limitations in notes/spikes/1680/README.md.
