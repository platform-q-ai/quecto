# #1614 ownership-preserving signaling

Parent resumed personally after fix-1614 explicitly stopped (no source changes or running tests in its handoff). The existing notes/1614-ownership-scope.md was discovered later and is preserved unchanged; its earlier RED status is not the status of this implementation. Issue metadata remains unchanged; no GitHub issue edit was attempted.

## Acceptance and reproduction

Scope is explicit local kill/cascade/shutdown vs local reaping, and bash cancellation while pipe drainage remains pending; no #1615 containment/discovery/non-Linux redesign or #1605 work.

Verified base bdccb5e0: local reaper calls child.wait before registry cleanup, removed entries survive awaited cleanup and shutdown drains before signaling. Bash held an outer numeric group guard until after stream collection, despite having reaped the shell.

RED commands (before behavioral changes):
- `cargo test -p quecto-agentic-harness --lib removed_entry_cannot_signal_after_its_reaper_finishes`: FAILED, `stale cleanup dispatched after reap: [5813]`.
- `cargo test -p quecto-agentic-harness --lib cancellation_during_pending_drain_keeps_group_leader_owned`: FAILED, `shell was reaped while pending drainage still allowed cancellation signaling`.

The first test intercepts signal dispatch in the actual cleanup path on its current-thread runtime. This is simulated reassignment/stale dispatch, NOT actual kernel PID reuse. The second owns an exited shell and a deliberately pending reader, without creating an escaped process. It safely checks whether the leader remains owned; no unsafe PID churn or unrelated signals occur. Reverting the ownership wait / restoring child.wait before drainage makes the corresponding test fail.

## Implementation

- Shared lease cloned with registry entries, captured before registry publication and passed directly to the local reaper. Each poll that can reap and each synchronous signal dispatch share a mutex; ready/error retires the lease before unlocking. Removed entries cannot revive authority.
- Kill/cascade and shutdown dispatch through that lease; rollback still signals before awaiting its exclusively held Child.
- Linux bash uses waitid(WNOWAIT) to observe shell exit, retaining the unreaped leader until pipe drainage completes. Cancellation's guard drops before Child; normal completion disarms before reaping. Timeout signals before reaping without retaining an armed outer numeric guard through drainage.
- Non-Linux retains existing child.wait and disarms immediately after reaping; no containment redesign is attempted.

## Same-class sweep / boundaries

- Reviewed all harness libc::kill, terminate_owned_process_tree and child.wait paths. Rollback remains exclusively owned until signaling completes. LocalProcessGroup has no production local assignment; its two-stage signal dispatch runs under the same lease when reached through registered cleanup.
- Forwarded descendants carry numeric PIDs reported by another harness, not a locally owned Child. This change does not establish cross-harness OS identity; that containment/ownership boundary is explicitly outside the narrowed local-reaper fix (#1615). Their prior signaling semantics are unchanged.
- Python lab wait/clear/cancel synchronization is separate #1605 work and deliberately unchanged. Session ownership uses signal 0 as a liveness probe, not destructive dispatch.
- Detach/no-kill does not invoke the signal lease; no new kill-on-drop policy was introduced for subagents.

## Validation so far

- Both RED tests now pass.
- Bash subset: 66 passed (including existing child/group cancellation tests).
- agent_cmd subset: 116 passed (including real owned-child parent/descendant termination).
- reaper subset: 3 passed.
- Full harness library: 3954 passed, zero failed/ignored.
- Strict harness all-target clippy with test-support: passed.
- Installed rustfmt/clippy for toolchain 1.97.1; cargo fmt --all passed.

No real kernel reuse/wrong-process signaling has been reproduced. Linux exit observation currently polls at 10ms, adding up to that completion latency. Review and publication gates remain pending.
