# Offline board fence

`quecto board-server fence --repo <clone> --board-id <16-hex> --url <http(s)>
[--url ...] [--remote <name>]` publishes one format4/MOVED commit to the existing
configured remote's `quecto/board`. The default remote name is `origin`. Missing,
unsupported or duplicate nonrepeat options refuse before Git composition; URLs
repeat in their supplied order and use the existing BoardMoved grammar/equality
without normalization. This command never bootstraps a missing branch, creates
a token, starts a listener, imports data or performs a deployment/cutover.

Use only after separately verified backup/import/server preparation and operator
approval. A successful fence changes where ordinary board reads and writes go:
they return the existing typed MOVED result rather than writing the old branch.
It is not a distributed SQLite/Git transaction or a server health guarantee.

The new commit's sole parent is the captured remote board head. The existing
fence-tree builder carries recognized folders and archive exactly and drops
strays like an ordinary write; old commits remain reachable. Caller author and
committer follow the existing board convention: `QUECTO_BOARD_COMMITTER` override
when supplied, else the explicit repository's allowlisted Git command/configured
committer. Git validates that it records this identity as given. No hardcoded
export identity or placeholder reader commits are used.

An exact already-fenced board/id/ordered-URL match returns `already` without
fresh identity lookup, new objects/commit or push (fetching existing objects is
permitted). A different destination is typed `store/moved`; malformed MOVED or
unsupported formats refuse without repair. JSON distinguishes published/head/
previous, already/head, conflict, refused, uncertain, unavailable and typed store
errors; any error exits1. It redacts diagnostics through the existing redactor.

Publication uses the existing private exact captured lease and remote outcome
judge, not run-evidence revisions. A competitor's winner is preserved; lost push
answers are reconciled against the actual remote. If that judge cannot establish
the outcome, `uncertain` never claims rollback or blindly retries/rebases/refences.
Inspect the remote before a separate operator decision. No unrelated refs move.
The repository's Git configuration and helpers retain their existing trust
boundary; this offline capability does not create a new credential policy.
