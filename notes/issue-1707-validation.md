# Issue 1707

Checklist: increasing queued left-panel elapsed time; lifecycle clearing; regression coverage;
standalone discoverable docs tool guide with configuration/defaults, status, troubleshooting,
and parallel spawn example. Admission itself worked in the reported ten-spawn smoke test.

Root cause: uds_admission_projection::admission_event_hook emits transition snapshots,
not periodic clock updates. The initial wait rounds to zero seconds. TUI admission handlers
cached formatted strings for master and children; animation ticks never recomputed them.
Fix uses local monotonic receipt anchors plus authoritative reported elapsed seconds,
refreshing all tabs on animation ticks without polling or changing broker scheduling.
Unknown elapsed stays unknown, additions saturate, new snapshots rebase, grants/run end/
disconnect clear waiting. Tool-owned spinner messages remain untouched by timer updates.

RED: cargo test -p quecto-tui queued_panel_timers --lib -- --nocapture failed with
left Some("waiting 0s"), right Some("waiting 3s") (paused Tokio clock, no second event).
Removing the animation projection restores this failure. Child disconnect regression
also failed before clearing cached child admission data on feed closure.
GREEN: targeted admission tests pass; full TUI library suite passed 2170 tests before
adding the disconnect test. Strict TUI all-target clippy passed.

Sweep: master footer, spinner, master panel and forwarded child panels share the fix.
Dated cooldown snapshot countdown is a separate sibling, filed as #1708; deliberately
not implying authoritative availability when a locally projected cooldown expires.

Independent review found child feed disconnect and identity rekey cleanup gaps.
Both now have failing-before/passing-after paused-clock regressions; rekey preserves
anchors using destination-wins policy. Final full TUI suite: 2172 passed. Dedicated
docs guide/TOC regressions: 14 passed. Guide configuration verified against production
configuration validation and broker CLI; docs example includes parallel tool calls.
