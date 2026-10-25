# Offline board backup

`quecto board-server backup --to <dir> [--board-id <16-hex>] [--state <absolute-dir>]`
backs up sorted canonical local boards from `<base>/board-server/<board-id>/board.sqlite3`,
the same file resolved for serve/import/export. An optional board id selects one
existing board. `--state` selects that absolute directory's `board.sqlite3` and
requires `--board-id`: a directory alone never determines a board's identity.
Empty, relative or repeated state options refuse before composition or I/O.
Native unbound NULL and matching-bound metadata are accepted; mismatched IDs or
provenance refuse. The command never starts a server, reads tokens or creates a
missing source directory.

Each invocation creates a fresh `<board>-YYYYMMDDTHHMMSSZ.sqlite` using SQLite's
online backup API and a pinned read-only transaction, including committed WAL
content. Sources are never opened writable or copied as database/WAL files.
Completed destinations pass exact-one-`ok` integrity validation before durable
no-replace publication. Prior snapshots also undergo schema, binding and SQLite
integrity admission; an engine-owned temporary API copy validates CHECK
constraints that the pinned SQLite engine omits on read-only schema loads.

The destination leaf must be owned by this user and mode0700; absent leaves
under an existing trusted parent are created privately. Legitimate existing
parent aliases are canonicalized and anchored. Canonical source/final symlinks,
hardlinks, overlap and detected identity replacement refuse. Supported Linux
fd-relative primitives, mode0600 owned partials and per-board advisory locks
protect cooperating producers; other platforms fail closed. This is not an
anti-malicious-same-UID authenticity or stat/unlink-race guarantee.

After verified publication, retain the latest snapshot per UTC day for the
newest14 represented dates **per board**, not a rolling336-hour window. Gaps
are permitted. Same-second collisions and backward/future-existing timestamps
refuse; the operator remains responsible for a truthful wall clock. Foreign
files are preserved. A prune failure keeps the new good snapshot and exits1;
publication uncertainty reports the candidate path and never claims rollback.

JSON contains every selected board, taken_at, published path, retained/deleted
counts, stage and typed error; unattempted deadline outcomes are explicit.
Exit0 requires all boards complete. Missing requested boards are `unknown_board`,
distinct from `no_targets` and a recognized database with `no_board` state.
Production budgets are120s copy,250ms busy,300s per board and1200s per invocation,
shared monotonic remaining budgets with interrupt/cancel/join validation. Scans
are limited to4096 entries,256 selected boards and256 candidates per board.
These are operational bounds, not hard OS-syscall preemption or native-history
count/RSS guarantees. Do not use this offline command for an unreviewed live
cutover or deploy; restore and transport policy are separate responsibilities.
