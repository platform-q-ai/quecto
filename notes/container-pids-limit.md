# Swarm containers dying with "fork rejected by pids controller"

Evidence (host journal, 2026-09-08/09, five occurrences): the kernel logs
`cgroup: fork rejected by pids controller in .../libpod-<container>` for a
swarm container that had run a parallel build/test (one consumed 39 min of
CPU in 16 min of wall time), then the founding agent panics at
`uds_shutdown.rs` with "OS can't spawn worker thread: Resource temporarily
unavailable (os error 11)", Podman reports the container died, and every
member agent disconnects at once. No OOM kills, no memory cap and no socket
failures were involved.

Two causes, both fixed here:

1. The create script set no `--pids-limit`, so Podman's default of 2048
   applied; threads count against it and an in-container `cargo test`
   exhausts it. The script now passes `--pids-limit 16384`
   (`QUECTO_CONTAINER_PIDS_LIMIT` overrides; `-1` defers to the user slice).
2. The termination teardown used `spawn_blocking`, which panics when no
   thread can be created. It now asks for a dedicated thread and, when the
   OS refuses one, runs the teardown inline: a slow stop, never a crash that
   takes the container's other agents down. Proof:
   `teardown_without_a_spare_thread_runs_inline_instead_of_panicking`.

Also in this change: the harness runtime keeps one warm blocking thread for
its whole life (`build_tokio_runtime`). Tokio's `spawn_blocking` only panics
when it has to create a thread and cannot; with a resident thread it queues,
so the other blocking calls on exit-sensitive paths (child reaper, cascade
cleanup, swarm lifecycle) stall instead of aborting the release binary. Bare
`std::thread::spawn` sites (query, provider reload, swarm supervisor,
runtime identity, admission process) still abort on EAGAIN and are a
follow-up. A kill script that cannot be started is now logged.
