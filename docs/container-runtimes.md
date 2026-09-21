# Container runtimes for subagents (`container_configs`)

Quecto can spawn a subagent into an isolated, script-managed environment
(e.g. a container) instead of a local child process. Quecto itself is
runtime-agnostic: it invokes an executable you configure, and that
executable owns Docker/Podman/devcontainer/whatever details. This is the
single canonical user document for the feature; the single canonical
reference runtime lives at [`scripts/container-runtime/`](../scripts/container-runtime/)
(`create.sh`, `exec.sh`, `inspect.sh`, `kill.sh` — see
"[The canonical reference runtime](#the-canonical-reference-runtime)").

## Spawning

The `spawn` tool's `container` field selects the launch adapter:

| `container` value | Behavior |
| --- | --- |
| omitted or `false` | Local child process (default, unchanged) |
| `true` | New container via this repo's `standard` config when its overlay declares one (#2035: no default elsewhere overrides it), else the config labeled `"default": true` |
| `{"mode": "new", "container_config"?: "...", "name"?: "..."}` | New container via the named config, with an optional container name |
| `{"mode": "existing", "ref": "C1"}` | Join the existing session environment `C1` via its retained `exec` script |
| `{"mode": "existing", "name": "review-env"}` | Join an existing environment by its (unambiguous) name |

Unknown fields are rejected. Runtime-specific fields (`branch`, `pr`,
`image`, ...) do not exist. `mode: existing` requires exactly one of
`ref`/`name`; unknown, ambiguous, stopped, or stale (kill pending/failed)
targets fail without guessing.

- There is **no `repo` field** (#1410): a container config is a complete,
  self-contained definition of a working context — its repository URL and any
  auth it needs are baked into the config's own argv, and the parent's
  location or checkout is irrelevant. A config with no repository is a
  **sandbox**: empty workspace, fully valid.
- `container_configs` are read from the launching agent's **effective
  configuration** (#2024): its global file with the trusted repo-local
  overlay of its working directory merged in, resolved fresh at every spawn
  — so container spawns normally need no `config` argument. An explicit
  `config` argument in the spawn call replaces both layers (as `--config`
  does) and must be an absolute path. To find the names in effect run
  `quecto config get --effective container_configs`.

A successful spawn returns a durable environment reference
(`environment_ref=C1`, `C2`, ...). Refs are allocated in the base
directory's registry, unique across every session sharing it, and never
reused — a stopped environment stays listed and its ref is retired.
The child then behaves like any other subagent: drive it with normal
`agent_cmd` operations over its direct or proxy endpoint. Every member of an
environment shares its reported workspace; each agent keeps its own agent
UUID, distinct from the environment's hidden UUID.

## Listing and killing environments (`agent_cmd`)

The environment registry is authoritative for `CN` ref, optional name,
runtime id, repository, workspace, retained script set, member agent
UUIDs, status, metadata, and last error. It is **durable per base
directory** (#2024 S4d, `<base_dir>/environments.json`, see "Environments
outlive sessions" below): refs are allocated there, unique across every
session sharing the directory, and every transition is written through.
Two session-level `agent_cmd` commands expose it (use `agent_id: "*"`):

- `get_containers` — lists every environment this session committed, and
  every environment earlier or concurrent sessions of the same base
  directory recorded (`restored: true`, `session` naming the creator,
  `config` the container config, `created_at` epoch seconds), with
  status `running`, `empty` (live, no members — every restored environment
  starts so: another session's members are not reachable here),
  `killing`, `stopped`, `cleanup-failed` (with its `last_error`), or
  `retained` (a swarm container kept because its owner has not ended the
  swarm — its coordinator was lost, or it holds an outcome nobody closed — with
  `metadata.retained` explaining which; see "Swarm environments live as
  long as the swarm"), plus workspace and members.
- `kill_container` with `ref` or `name` — takes the environment's
  exclusive kill claim, asks every member agent to shut down over its own
  control edge (the `shutdown` protocol; a container coordinator's harness
  settles its in-container descendants itself, and the host never signals
  a pid inside the box), then runs the environment's retained `kill` argv
  exactly once and commits `stopped` only after the script succeeds. Its
  JSON result includes the environment ref, up to 20 member agent
  ids/names (with `omitted_agents` when capped) and `settled`, how each
  member ended before the kill ran (`graceful`, `fallback`,
  `already-exited`, `unobserved` — asked, but no exit was observed within
  the bound, so the retained `kill` is what ends it — or `joined`). A
  member whose end cannot be settled (a termination another path already
  owns that never settles, or a locally owned process that survives the
  fallback) withholds the retained `kill` and persists a retryable
  `cleanup-failed` state naming the member; a failed `kill` script does the
  same. Run `kill_container` again to retry: only the members still
  recorded are asked again, and the retained `kill` never runs twice under
  one claim. Latency: a member is asked over its edge with a 5 s
  acknowledgement bound and, once acknowledged, given up to 25 s (the
  owned-handle ladder plus slack) for its exit to be observed before it is
  compensated `unobserved`, so a `kill_container` whose members are
  unreachable or slow to exit can take up to ~30 s per member before the
  retained `kill` runs; a member that exits promptly settles in
  milliseconds.

When the final member of a live environment exits or is killed, the same
retained `kill` operation runs exactly once (concurrent final exits cannot
double-kill). Script sets without a configured `kill` fall back to the
retained `cleanup` argv for final-member teardown; `kill_container` itself
refuses such environments up front, leaving every member untouched.

### No host signal ever enters a container

A container member's harness — and everything it launches inside the box —
is ended over the protocol only: the host asks it to `shutdown`, observes
the exit through its snapshots, and lets the retained `kill` argv end the
box. The host holds no process handle for anything inside a container and
records no pid for a descendant reported from inside one (a merged row's pid
is always `0`), so pids 1 and 2 inside a box — `podman-init` and the
coordinator — can never be targeted from outside. This is ratcheted by
`quecto-agentic-harness/tests/architecture/teardown_authority.rs` (the
process-effect allowlist) and proven by running the harness's own full BDD
suite inside a `quecto-dev:local` container whose pid 2 blocks and logs every
signal it receives (see the #1940 record below).

#### In-container proof record (#1925 method, #1940 run)

Method (issue #1925): the harness's own full BDD suite runs inside a
`quecto-dev:local` container as the child of a **signal-logging pid 2** — a
Python wrapper that blocks `SIGTERM`/`SIGINT`/`SIGHUP`, runs the suite as its
child (with the mask unblocked for the child), and logs every signal it
receives from a `sigwaitinfo` loop with `si_pid` and the sender's `cmdline`.
Pid 2 is where a swarm coordinator's harness sits, so any registry, fixture or
descendant pid the suite ever targeted would receive the signal there.

- **Revision:** `ec29e9902b96fdab2534a0f51dd82bdbace0f5e2` (the #1940 PR head at the time of the run; the commits that record it follow)
- **Image:** `quecto-dev:local`, id `b1f87e8917502e1963979a0ed43fd7866427c961c26aac0d1576a17774b63662`
- **Command:** `scripts/bdd-in-box/run.sh` (committed with the pid 2
  wrapper `scripts/bdd-in-box/pid2_signal_log.py`), which runs exactly:

  ```bash
  podman run --rm --init --name quecto-bdd-in-box \
    --userns=keep-id --pids-limit 16384 --user 1000:1000 \
    -v <repo>:/src -v quecto-bdd-in-box-target:/tmp/target \
    -v <scratch>/home:/home/dev -v scripts/bdd-in-box/pid2_signal_log.py:/pid2_signal_log.py:ro \
    -e CARGO_TARGET_DIR=/tmp/target -e HOME=/home/dev -e TMPDIR=/home/dev/tmp \
    -e PID2_SIGNAL_LOG=/home/dev/pid2-signals.log -e RUST_LOG=warn -w /src \
    quecto-dev:local python3 /pid2_signal_log.py \
    bash -c 'status=0; for i in 0 1 2 3; do echo "=== shard $i/4 ==="; \
      QUECTO_BDD_SHARD_INDEX=$i QUECTO_BDD_SHARD_TOTAL=4 \
      cargo test --workspace --features quecto-agentic-harness/test-support --bins --test bdd || status=1; done; exit $status'
  ```

  The suite runs as four sequential shards (one `bdd` process each) under
  the same pid 2 because one process running all ~1600 scenarios exhausts the
  container's 16384-pid cgroup: the BDD steps leak forgotten tokio runtimes
  (`std::mem::forget(runtime)`, 128 sites), ~19 threads per scenario, and a
  single-process run stalled at 16378 threads with `Cannot fork` — tracked
  by #1959, whose close re-runs the proof as one process.
- **Log:** `/var/tmp/q1940/proof/final-run.log` (10 700 lines) and
  `/var/tmp/q1940/proof/final-pid2-signals.log` on the machine that ran it.
- **Counts:** 4 shards, **1609 scenarios passed, 0 failed** (393 + 376 + 394
  + 446), 8021 steps passed; child exit status 0; runtime 15:18:26 →
  15:26:41 (+ a 4 min cold build).
- **Signals to pid 2: 0** (`SIGNALS_TO_PID2 0`); the wrapper survived with
  pid 2 for the whole run.

Found by this proof and fixed in the same PR: a launched child whose launcher
died and whose stderr pipe therefore had no reader hung on its first log line
when `RUST_LOG` was set (`tracing-subscriber` reports a failed write with
`eprintln!`, which panics on a broken stderr inside the task that was about
to run the parent-loss shutdown). `RedactingWriter` now swallows sink errors
and the subscriber's internal-error reporting is off; the
`subagent_delegated_subtree.feature` SIGKILL scenario reproduces the
configuration on the host and fails without the fix.

### Swarm environments live as long as the swarm (#1924, #2070)

A swarm's container lives as long as its swarm, and no longer. A swarm ends
only when its owner says so: the supervisor outside the swarm closes the run
into its outcome (`swarm_control close`), or the owner explicitly leaves
everything it owns behind — delete-all, or a session transition that
succeeds (a `/resume` proves its target before the fleet is settled, so a
refused one ends nothing). Then the final
member's exit runs the retained `kill` like any other container: the box,
its checkout and its board go; nothing is kept.

The one exception to final-member teardown is a swarm its owner has NOT
ended. Whenever the member whose exit empties the environment was the
coordinator of a created run that has not been closed — `running`, `paused`,
paused holding an outcome, or `cancelled` by the coordinator agent itself —
the cascade still marks the member (and its descendants) exited, but the
environment record moves to `retained` instead of running the retained
`kill`; the container and its state directory stay so the run can be
resumed, and an explicit `kill_container` removes them. That holds for the
coordinator's own socket closing, for an `agent_cmd kill` of that one
coordinator agent, and for the master's own process shutdown — which can be
a crash (a termination signal, its last client gone, a lost parent), so it
never counts as the owner's word. `metadata.retained` says which case
applies:

- **Orderly end, not yet closed** — the run was already paused holding an
  outcome, or cancelled by its coordinator, when the member went away: `run
  ended: <outcome>; environment retained for inspection, kill_container to
  remove` or `run ended: cancelled; ...`. The run is untouched: no quarantine
  and no `resume_blockers` entry. (A run the supervisor already closed into
  its outcome is not retained at all.)
- **Loss** — the run was `running`, or paused with no outcome, when the
  coordinator's socket closed: the coordinator is quarantined exactly as an
  in-swarm reconcile treats a lost harness, the run is paused holding
  `failed`, and the control receipt's `resume_blockers` names the
  coordinator that must be relaunched before a resume can proceed;
  `metadata.retained` reads `swarm coordinator '<id>' lost its connection;
  run paused holding failed; ...`. Observation and quarantine are one store
  operation, so a run that ends first (for instance the store's own expiry
  check ending it as `budget-exhausted`) is reported as an orderly end.
- **Supervisor kill** — `agent_cmd kill` of the coordinator, or the master's
  shutdown, while the run has not been closed: `coordinator killed by
  supervisor; run <status>; ...`. Nothing is quarantined; the run is left as
  it was.

The decision is made from the session that launched the container by
reading the coordination store at `metadata.checkout` (see the `create`
contract). A create result that omits `metadata.checkout` (older or
third-party script sets) is warned about at create time and the host then
probes `<workspace>/repo` and `<workspace>` for a store. Either way the
path must be absolute, `..`-free and — after resolving symlinks on both
sides — under the environment's workspace; anything else is ignored, as if
no store existed. An environment without a store, or whose run is the
bootstrap placeholder, keeps the ordinary final-member teardown, while a
store that exists but cannot be read (contended, corrupt) also retains — a
destroyed box cannot be recovered, a retained one can always be killed.

A retained environment is for inspection: exec into the container, read the
board and checkout, read the member logs (see the adapter section). A join
(`{"mode":"existing","ref":"C1"}`) is admitted but does not revive the
environment (relaunching the coordinator against the surviving store is not
wired yet, and the paused run refuses activation); a joiner that exits or
rolls back leaves the record `retained`. A retained environment outlives the
master session that produced it (the master's shutdown retains it too), and
its record outlives the harness: remove it with `kill_container` from that
session while it lives, from any later session of the same base directory
(`get_containers` lists it `retained` with `restored: true`;
`kill_container` by ref or name), from the shell with `quecto container
kill <ref|name>`, or manually with the adapter's `kill.sh` (or `podman rm -f
quecto-<environment_id>` plus its state directory) — after which `quecto
container gc` removes what is left.

## Environments outlive sessions (#2024 S4d)

Every harness keeps its environment registry in
`<base_dir>/environments.json` (0600, written atomically under the same
`<base_dir>/locks/` `flock` the configuration writer uses): for each `CN`
ref the runtime id, name, container config, retained `exec`/`kill`/
`cleanup`/`inspect` argv, status, workspace, repository, metadata, last
error, the creating session's key and the creation time. **Members are not
stored** — they belong to the session that launched them. Refs are
allocated in the file, so two sessions on one base directory never mint
the same `C7`, and a ref is never reused after a restart.

At startup a top-level session **restores** the file: each record is
checked against the runtime through its retained `inspect` — a
`running`/`cleanup-failed` record whose inspect says `dead` is marked
`stopped` with `container not found at restore …` as its last error
(never silently dropped); one whose inspect cannot be run is kept as
recorded and reported unverified on stderr; a record found `killing` is
reported as *kill in flight (its session may be live and settling it)*
and left as it is — nothing here can tell whether that session is dead —
and an explicit `kill_container` / `quecto container kill` from the new
session retries it. A **`retained` record is never relabelled** (#1924):
under the shipped adapter its container has *exited* by design (the
coordinator was PID 1, the state directory holds the board, checkout and
unpushed work), so a dead inspect is reported as *retained: container
exited; only container kill ends it* and the file is left alone —
`ls` lists it as `retained`, `gc` keeps it, and only `container kill` /
`kill_container` moves it to `stopped`. A **`running` record whose
container is gone but whose checkout still hosts a swarm run that has
not ended** — the master exited (or was killed) *before* its
coordinator, so nobody finalized the member; the coordinator's harness
then ran its parent-loss shutdown and the box exited with the board,
checkout and unpushed work inside — is relabelled **`retained`**, not
`stopped`, with `metadata.retained` reading `run <id> unfinished
(<status>); container exited; environment retained for inspection,
kill_container to remove` (what the finalizer would have recorded had
the master seen the coordinator go; `ls` prints the note `<ref> retained
at restore: …`); a store that exists but cannot be read retains too. An
ended run, the bootstrap placeholder or no store leaves it `stopped` as
before. A record an **older build's restore relabelled `stopped` while
it was retained** (recognisable by its own `metadata.retained` under
`stopped` with the restore's `container not found at restore …` last
error — an explicit kill clears that error) is restored to `retained`,
so `container kill` can end it. Restored records carry `restored: true`
and `session` in `get_containers`. A spawned child
journals its own creates but is not seeded (its parent shows the fleet).
A session that started while the file was unreadable retries the read on
its next lookup (`get_containers`, a join, a kill), so a document
repaired in place is seen and seeded without a restart — the stale
diagnostic disappears from the next listing.

**What the file is trusted for.** A record is trusted as written: the
retained `exec`/`kill`/`cleanup`/`inspect` argv are run verbatim from the
record, not re-resolved through the configuration in effect now (the
config's scripts may have moved or changed since; the record's are the
ones that created the environment). `config` (the config's name) and
`created_by` are provenance for the listing, not a re-lookup. The one
exception is the standard bundle's scripts: an `init`-materialised
`.quecto/container/*.sh` is re-judged against the embedded bundle before
a join or kill runs it (S4e integrity), whichever record names it. The
file is 0600 under the base directory — whoever can write it can run
argv as the harness user, exactly as with the configuration file.

**Durable names are check-then-act.** A create refuses a `name` that
still names a live environment (this session's or a restored one) by
reading the registry *before* the create script runs; two sessions
creating the same name at the same moment can both pass that check, and
the name is then ambiguous — `mode: existing` by name fails ambiguous and
`kill_container` by name asks for the ref. Refs are allocated under the
file lock and never collide. While the registry file is unreadable
(corrupt, a newer version) the session starts with an empty registry,
**every container create is refused** (a ref minted from memory could
collide with one a live session holds) and `quecto container ls|kill|gc`
refuse in the same words; a session's later journal writes are reported
on stderr when they fail, never retried silently. Writes to a record
another session created (a joiner's inspect metadata, its kill) are
compare-and-set on the status this session last saw on file: when the
creator has moved the record on meanwhile (say to `retained`), the
creator's state stands and nothing is reverted.

What a new session can do with a restored environment:

- **list** — `agent_cmd get_containers` (`agent_id: "*"`), or from the
  shell `quecto container ls` (live ones; `--all` includes `stopped`):
  `REF NAME CONFIG STATUS REPOSITORY CREATED-BY AGE`.
- **join** — `spawn {"container": {"mode": "existing", "ref": "C1"}}` (or
  `"name"`) while it is `running`/`empty`/`retained`; the retained `exec`
  runs. This is the **concurrent-session** case: the environment's
  container is still up because the session that created it is still
  alive (or the environment is `retained`, whose box has exited and
  which a join does not revive). Under the shipped adapter a member
  harness runs its parent-loss shutdown and exits when the harness that
  launched it dies, and the container exits with it — so once the
  creating session is gone an ordinary environment is `stopped` at the
  next restore (its container gone), not joinable. A joiner leaving a
  restored environment **never tears it down** (its creating session may
  still hold members this session cannot see); only an explicit kill
  ends it.
- **kill** — `agent_cmd kill_container` with `ref`/`name`, or `quecto
  container kill <ref|name>`: no members of another session are asked
  (none are recorded), the retained `kill` runs once, `stopped` is written
  through. The creating session, if still alive, sees its member die and
  runs its own final-member kill after yours — the shipped scripts are
  idempotent.
- **collect** — `quecto container gc [--dry-run] [--name <config>]`
  removes **orphans**: environments whose container is exited or unknown
  to the runtime **and** that the registry either does not record or
  records `stopped`. Its scope is the config's `--state-dir` (from its
  create argv, compared canonically — a symlinked spelling is the same
  root), under which everything is judged, plus — for a record of that
  config whose workspace lies under the root its *own* retained
  `cleanup` argv names — that record's directory alone, removed through
  that record's cleanup alone. A record's workspace never widens the
  scan by itself: a record lying anywhere else is reported *outside this
  config's state dir; not collected* and nothing is scanned or run for
  it (a doctored `workspace_path` cannot point the config's cleanup at a
  foreign directory). It lists containers through the config's
  `inspect --list` (one JSON object per container the create script
  labelled, so exited containers whose directory is already gone are
  found too) and removes through the record's retained `cleanup` or the
  config's `cleanup` — the harness itself names no runtime. A `running`,
  `killing` or `cleanup-failed` record is always **kept** (with the
  reason, pointing at `container kill`; one seen nowhere — no directory,
  no container — is reported the same way, never silently skipped, and
  a `stopped` one seen nowhere has its record forgotten); a `retained`
  record is kept **whatever the runtime says of its container** (exited
  is its normal state; only an explicit kill moves it to `stopped`,
  after which its leftovers are the collector's); so is any state dir
  whose container runs (a directory under a root the record's *own*
  cleanup names is judged by that record's own retained `inspect`, not
  the config's), and any state dir younger than fifteen minutes with no
  container recorded yet (a create may be in flight — a directory whose
  age cannot be read counts as young). What would otherwise be collected
  — a `stopped` record's directory, or an unrecorded one — is read last
  for the coordination store its checkout may host (`workspace/repo`,
  then `workspace`): one hosting a **swarm run that has not ended**, or a
  store that cannot be read, is **kept** whatever the registry says —
  `… hosts swarm run <id> (<status>); end the run … before it can be collected` — the
  board and checkout are the run's; an ended run, the placeholder or no
  store is collected as before. A record an older build relabelled
  `stopped` while retained is kept likewise (`was retained; relabelled
  by an older build`; the next `ls`/`gc` restores it to `retained`, after
  which `container kill` ends it). `--dry-run` prints the same judgement with
  **no effect at all** — nothing on the host, and nothing written to
  `environments.json` (the corrections a restore would write are
  previewed on stderr as *would be recorded …; not written*); the report
  lists what was (or would be) removed, what was kept and why, and every
  failure.
- **an unrecorded running container** — a harness that died between the
  create script and the journal write (or whose `environments.json` was
  moved aside) leaves a container the runtime reports running and no
  record names. `gc` keeps it (its container runs) and `ls` cannot show
  it; end it by hand: `podman rm -f quecto-<environment-id>` (docker
  likewise) and remove `<state-dir>/<environment-id>` — after which
  `gc` collects nothing further for it. Such a container does not stay
  up for long on its own: the member harness inside it holds a
  launch-bound connection to its parent and runs its parent-loss
  shutdown — and exits — when that parent dies, so the container exits
  too and `gc` then collects it as an unrecorded exited orphan (unless
  its checkout hosts an unfinished swarm run, below). Only while the
  parent lives, or for a script set whose member ignores parent loss,
  is a hand kill needed.

**Moving `environments.json` aside restarts the refs** (the next create is
`C1` again) and forgets every environment it recorded: the containers
themselves keep running, but no session lists, joins or kills them by ref
any more — they become orphans for `quecto container gc` once they exit
(or for `podman rm -f` by hand). That is the intended outcome of moving
the file aside; repair it in place instead when the environments matter.

`quecto container ls|kill|gc` resolve the container config the way the
doctor and `spawn` do (the working directory's effective configuration, or
`--config <file>`), restore the registry for their own run, and never
touch a live environment.

## Configuration

> **Migrating from pre-#1410 configs:** the `container_scripts` key was
> renamed to `container_configs` and its shape changed (flat map of named
> container configs; the default is labeled `"default": true` on one entry;
> repositories are baked into each config's `create` argv via `--repo`).
> A config still containing `container_scripts` fails to load with an error
> pointing here — rename and reshape the section.

```json
{
  "container_configs": {
    "quecto": {
      "default": true,
      "create": ["/abs/path/to/create-script", "--repo", "https://github.com/platform-q-ai/quecto"],
      "cleanup": ["/abs/path/to/cleanup-script"],
      "exec": ["/abs/path/to/exec-script"],
      "kill": ["/abs/path/to/kill-script"],
      "inspect": ["/abs/path/to/inspect-script"]
    },
    "sandbox": {
      "create": ["/abs/path/to/create-script"],
      "cleanup": ["/abs/path/to/cleanup-script"]
    }
  }
}
```

Each entry is a named **container config**: a complete, self-contained
definition of a working context. Exactly one entry must carry
`"default": true` — the config `container: true` selects when the checkout's
overlay declares no `standard` entry (a repo's `standard`, written by
`quecto container init`, is its default by rule: `container: true` selects
it whatever the global file or another overlay entry labels, even after
`quecto config unset --local container_configs.standard.default` removed
its own label — a raw edit of the overlay un-trusts it instead, and the
label can only go while another entry carries one, since a merge with no
default is refused at load); zero or multiple
default labels fail at config **load** time with an error naming the
configured entries. Operations are argv arrays executed directly — no
shell interpolation — and the repository (with any auth it needs) is part
of the config's own argv (`--repo` for the shipped scripts), never
something Quecto resolves or passes. `create` and `cleanup` are required;
`exec` (joining), `kill` (explicit stop), and `inspect` (post-mortem) are
optional but needed for `mode: existing`, `kill_container`, and death
diagnostics respectively. Missing, unknown, empty required, or unsafe
(empty/NUL argument) configuration fails before any script runs, and
selection errors enumerate the available config names so an agent can
offer the menu. Agents see the names before spawning in two places (#2024
S4c): the `spawn` tool description carries one bounded roster line
(`Available container configs: <name> (default, repo-bound|global), …`,
at most 120 characters, the tail folded into `+N more`; a withheld overlay
adds `(repo overlay untrusted — run quecto config trust)`; one entry alone
over the budget is cut with an ellipsis), re-rendered whenever a
configuration layer or the trust record changes (the tool definitions are
rendered for every model call, so the line is cached against the files'
metadata); and
`agent_cmd {"agent_id":"*","command":"get_container_configs"}` returns the
same effective set with detail, live at each call:
`{"container_configs":[{"name","default","source":"overlay"|"global","repository","problem","joinable"}],"overlay_withheld":bool,"diagnostics":[…]}`
— the `container: true` default first; `default` is what a launch would
honour (a repo-bound `standard` whatever the labels say, none while the
overlay is withheld, none when more than one entry is labelled and no
repo-bound `standard` exists, never an entry with a `problem` — a missing
or unsafe argv, diagnosed in `diagnostics`; an unlabelled repo-bound
`standard` is diagnosed with the remedy); `source` says which layer declared the entry,
`repository` is the create argv's `--repo` (`null` for a sandbox),
`joinable` whether the config carries an `exec` argv for
`{"mode":"existing"}` joins. Operators
see the same set with `quecto config get --effective container_configs`.

The container config in effect when an environment is **created** is
retained with the environment: later joins, kills, and inspects use the
retained `exec`/`kill`/`inspect` argv even if the labeled default changes
afterwards.

### Binding a repository to a container config

A repository says "my containers use config X" through its repo-local
overlay `<checkout>/.quecto/config.json` — the same overlay every other
section uses (see the `config` docs page and the harness README), with the
same `container_configs` shape. The overlay merges **entry-wise** over the
global file: a local entry with `"default": true` un-defaults every global
entry, a same-name local entry shadows the global one, and other global
entries stay selectable by name. The merged set must still carry exactly
one default; a write that would break that is refused before it lands.

**Preconditions**: a script set with absolute paths (the shipped adapter or
your own), and the repository URL the container should clone.

**Do** — from the checkout (writes the overlay and records its trust):

```
quecto config set --local container_configs.app '{"default":true,"create":["/abs/create.sh","--state-dir","/abs/state","--repo","https://github.com/org/app"],"cleanup":["/abs/cleanup.sh"],"exec":["/abs/exec.sh","--state-dir","/abs/state"],"kill":["/abs/kill.sh","--state-dir","/abs/state"],"inspect":["/abs/inspect.sh","--state-dir","/abs/state"]}'
```

Then, from an agent started in that checkout, `spawn` with
`container: true` creates the container with `app`'s `create` argv (unless
the overlay also declares `standard`, which wins by rule); from any other
directory the global default still applies. The binding applies
only to runs started *without* `--config` (an explicit `--config` file
replaces both layers, as a spawn `config` argument does) and never inside
a container child: the child is started with the global file and has no
checkout overlay to bind, so a container spawned from inside a container
selects from the global entries.

**Verify**: `quecto config get --effective container_configs` shows `app`
as the one default; `quecto status` shows `Overlay: … (trusted)`; a spawn
result names `environment_ref=C1 container_config=app` and
`agent_cmd get_containers` lists the repository the create script reported.

**Rollback**: `quecto config unset --local container_configs.app`, or
delete `.quecto/config.json` to drop the whole overlay.

**Trust**: the overlay is applied only when its exact content is recorded
in `<base_dir>/config-overlay-trust.json` (canonical path + SHA-256).
`quecto config set` records trust for what it writes; an overlay written
by hand, or committed by someone else, needs `quecto config trust` from
the checkout after review — an explicit, non-interactive command an agent
can run. Until then the overlay contributes nothing to container spawns.
`container: true` is **refused** when the withheld overlay could have
changed the default — it is a symbolic link, unparseable or failing the trust checks, or an untrusted
document that declares `container_configs` (the default it labels is
unknown, so an implicit selection must not quietly land in the global
one) — with a tool error naming the overlay and `quecto config trust`. An
untrusted overlay that declares no `container_configs` (one that only pins
`agents.defaults.model`, say) cannot have changed the set: the global
default launches and the result carries the diagnostic as a warning. A
named `container_config` launches from the global set and its result
carries the same diagnostic under `Configuration diagnostics:`; a name
only the withheld overlay defines is `unknown container config` with the
diagnostic appended to the error. The spawn also prints the diagnostic to
stderr, as `quecto status` does. There is
no separate container trust record and no `[y/N]` prompt on the spawn
path any more (the pre-#2024 `container-config-trust.json` is not read;
approve such an overlay once with `quecto config trust`). Container
repository and auth semantics remain self-contained in the selected
config's argv; the parent's checkout supplies the *selection*, never the
source.

## Endpoints and liveness (direct vs proxy)

A `create`/`exec` result must carry **exactly one** endpoint:

- `"socket_path": "/path/to/child.sock"` — a direct UDS endpoint the
  parent connects to; or
- `"socket_proxy": {"argv": ["/abs/path/to/proxy", "args"...]}` — a
  validated argv the parent runs once per connection. The proxy process
  speaks the child protocol on its stdio (typically bridging to a socket
  that is only reachable inside the environment). Unknown keys, an empty
  argv, or empty/NUL arguments are rejected.

Results carrying both or neither endpoint fail the launch with rollback.
In proxy mode Quecto binds a private parent-side bridge socket and never
connects to (or falls back to) the direct socket path that was passed in
the child's CLI args. A bridged connection is torn down — including its
proxy process — as soon as either side closes, so dropped probe or
one-shot command connections never leave proxies (or their connections
into the child) lingering.

Proxy readiness probes the bridge across the launch's readiness budget:
a probe that reads EOF (the proxy could not reach the child yet) is
retried; the endpoint is bridge-ready once a probe survives a quiet
window. For proxy launches with an initial task, an initial prompt send
failure is folded back into the same launch readiness retry budget before
rollback, so a proxy that connected before the child accepted commands
gets another chance without ever falling back to a direct socket or
polling lifecycle state after launch. Because the child protocol cannot
prove whether an ambiguous lost acceptance response was observed after the
child queued the prompt, proxy-managed initial tasks must remain
idempotent: a retry may resend the same initial task if delivery succeeded
but the acknowledgement did not return before the retry deadline.

After readiness the parent holds one persistent monitor connection to the
endpoint. EOF or connection reset on it IS the child's death signal — no
lifecycle polling, no wrapper process. That same connection is the child's
**launch-bound parent control connection** (#1935): the parent presents the
launch capability as its first frame and the child treats the loss of that
one connection as "my parent is gone" and shuts itself down. For this to
hold end to end through a proxy, the proxy argv must exit (closing its
connection into the child) when its stdin reaches EOF — the parent's side of
that pipe closes whenever the parent dies, SIGKILL included. Every proxy
process is owned by the parent's child supervisor; when the bridged
connection ends its stdin is closed first and it receives a signal only if it
does not exit by itself. On death, the environment's
retained `inspect` runs exactly once for that member (repeated EOF/reset
signals do not re-run it), the authoritative environment record is
updated before the member is removed, and the exit feeds passive
completion notes, snapshots, and live events exactly like a local child. Because no exit status exists for a
script-managed death, lifecycle observers report how the death was
observed — `connection_closed` (EOF or reset) or `never_reachable` (the
endpoint never accepted the monitor
connection) — never a fabricated `exit_code_0`.

## Script contract

### `create`

Invoked as:

```
create-argv... -- <child-binary> <child-args...>
```

The repository is part of `create-argv` itself — the shipped scripts take
`--repo <url>` there, and a config that omits it is a sandbox (empty
workspace). Quecto passes **no source information**; environment variables
provided to the script:

| Variable | Meaning |
| --- | --- |
| `QUECTO_CONTAINER_CONFIG` | Name of the selected container config |
| `QUECTO_CONTAINER_ENVIRONMENT_REF` | The minted session ref (`C1`, ...) |
| `QUECTO_BASE_DIR` | Parent agent's base directory (set only when the parent has one) |

The script must start the child (`<child-binary> <child-args...>`)
**exactly once** — Quecto never starts a local fallback child — and print
exactly one JSON object to stdout:

```json
{
  "environment_id": "runtime-owned-id",
  "workspace_path": "/path/to/workspace",
  "metadata": {},
  "socket_path": "/path/to/child.sock"
}
```

`environment_id`, `workspace_path`, and `metadata` are required, plus
exactly one of `socket_path` or `socket_proxy` (see "Endpoints and
liveness"). Extra JSON data after the object is rejected. A non-zero exit
or an invalid contract fails the launch and rolls back. **Logs and
diagnostics go to stderr**: the last 4 KiB of the script's stderr (with
terminal escapes and control characters removed) are appended to the tool
error after the exit status — `script-managed create failed with status
exit status: 6: … image quecto-dev:local is not present …` — and echoed on
the parent harness's stderr, so a `die` message reaches both the model and
the operator (#2024 S4b). The same applies to `exec`, and to the retained
`inspect`, `kill` and `cleanup` scripts (a failed cleanup is logged with
its tail).

A create script may also implement the preflight mode the doctor drives:
invoked with `--preflight-only` appended to its configured argv and **no**
child command after `--`, it evaluates every prerequisite without creating
anything and prints one line per check on stdout,
`status<TAB>check<TAB>detail<TAB>remedy` with `status` one of `ok`,
`warn`, `fail`, exiting non-zero when any check failed. The official
Docker/Podman adapter implements it (see "Troubleshooting"); a script that
does not is reported as such by `quecto container doctor`.

The doctor **fails closed** on that report: a non-zero exit with no `fail`
line (the script died mid-report — a usage error after its first checks, a
stuck runtime) is an error carrying the script's stderr tail, never the
healthy-looking prefix it managed to print; a tab-separated line whose
status word is unknown or that has fewer than three fields (`fail<TAB>image`
cut short) refuses the whole report naming the line; and a report the
script exited 0 with must contain the checks every shipped script makes on
every run — `jq`, `git`, `repo`, `state-dir` (the last check of both
scripts) — or it was cut short. A report that already carries a `fail`
line and exits non-zero may be partial (the script stops at the first
failure); the failure shown is real. Lines without a tab are log noise and
ignored.

Never embed credentials in a `--repo` URL (`https://user:token@host/…`):
the scripts show every URL they name with its userinfo replaced by `***`
(preflight lines, log lines, `metadata.repository`), and the harness
redacts the same shape in any stderr tail, doctor header or spawn error —
but the token still sits in the config file and in the clone's remote.
Use an `ssh` key or `gh auth login` (the `gh auth git-credential` helper
travels into the container) instead.

The create result must contain exactly these fields — unknown keys are
rejected. When the config cloned a repository, the script should report it
as `metadata.repository`: that is how `get_containers` listings and the
TUI learn the source truthfully (sandbox configs report none and list an
empty repository). A script set that can host a swarm should also report
`metadata.checkout`: the members' working directory and swarm checkout root
(`QUECTO_SWARM_CHECKOUT`), identity-mounted so the session that launched the
container can read the coordination store at
`<checkout>/.quecto/swarm.sqlite` after the members' sockets are gone and
keep the environment instead of destroying a resumable run (#1924).

### `exec`

Invoked to add another agent to an existing environment:

```
exec-argv... -- <child-binary> <child-args...>
```

Environment variables: `QUECTO_CONTAINER_CONFIG` (the retained container
config's name) and `QUECTO_CONTAINER_ENVIRONMENT_ID` (the runtime `environment_id`
reported by `create` — not the session `C` ref). The script must start the
child inside the existing environment exactly once and print exactly one
JSON object:

```json
{
  "metadata": {},
  "socket_path": "/path/to/joined-child.sock"
}
```

The exec result carries a `metadata` object plus exactly one of
`socket_path` or `socket_proxy` (same endpoint contract as `create`);
unknown keys are rejected.

Known limitation: the exec result carries no process handle, so if a join
fails after the script started the child (socket never ready, registration
refused), Quecto cannot terminate that process individually. It keeps
running inside the environment until the environment's retained `kill`
(or final-member `cleanup` fallback) tears the environment down. Exec
scripts should therefore make the started child exit on its own when its
socket is never connected to.

### `kill`

Invoked with `QUECTO_CONTAINER_ENVIRONMENT_ID` set to the runtime
`environment_id`. Runs exactly once per successful stop — from
`kill_container`, from `delete_all_subagents`, when the final member exits,
or when the parent harness receives SIGTERM/SIGINT. A non-zero exit leaves
the environment in a retryable `cleanup-failed` state with the script's
stderr preserved as the last error.

The signal case matters because an environment's processes live outside the
parent's process group: the TUI's ordinary exit (Ctrl-D) terminates the
harness by signal, and nothing but the harness running this script can reach
the environment. The harness therefore tears down every subagent and
environment on the signal before it exits, and the TUI's exit budget (two
seconds) bounds how long the kill script may take. The exception is a swarm
its owner has not closed (#1924, #2070): a signal can be a crash as easily as
an exit, so that environment is `retained`, not killed — see "Swarm
environments live as long as the swarm".

### `inspect`

Invoked with `QUECTO_CONTAINER_ENVIRONMENT_ID` set to the runtime
`environment_id`, exactly once per dead member, after the member's death
is pushed (EOF/reset) and before the member is removed from the
environment record. A parent-initiated `kill_container` is not a
post-mortem: members terminated by it are not inspected. The inspect
subprocess is bounded (5s); on timeout it is killed and an inspect
failure is persisted, keeping the retained argv for retry. It must print
exactly one JSON object:

```json
{
  "status": "dead",
  "metadata": {"cause": "oom-killed"}
}
```

`metadata` (required object) is merged over the environment's stored
metadata and becomes visible via `get_containers`; `status` (optional) is
recorded as `inspect_status` — and judged by the registry restore
(#2024 S4d): `running` keeps the record, `dead`/`exited`/`removed`/
`stopped` marks it stopped, anything else (or a failed inspect) leaves it
unverified. A missing environment directory is not an error: the shipped
Docker/Podman script asks the runtime about `quecto-<environment_id>` and
reports `running` (cause `state-dir-removed`) or a truthful `dead` (cause
`environment-removed`); the host-local reference reports `dead`. With
`--list` appended (no environment id) the script prints one JSON object
per line for every environment the runtime knows **under this state
root** (the create labels each container `quecto.state_dir=<root>`) —
`{"environment_id": "env-…", "container": "quecto-env-…", "status":
"running"|"dead"}`, `dead` only for a container the runtime calls
exited/dead/stopped — which is how `quecto container gc` finds exited
containers whose directory is gone; a script set without `--list` cannot
serve the collector (the error says so). The result is parsed with the same strict
wire rules as `create`/`exec`: exactly these fields — unknown keys,
trailing JSON data, and non-UTF8 output are rejected. A non-zero exit or
invalid contract persists an actionable inspect error on the environment
(surviving later successful cleanup) while keeping the retained `inspect`
argv; a later member's successful inspect supersedes and clears the
stale inspect error (a `cleanup-failed` kill error is never cleared by
an inspect).

### `cleanup`

Invoked with `QUECTO_CONTAINER_ENVIRONMENT_ID` set to the runtime
`environment_id` being destroyed (note: a different identity than the
session `C1` ref the create script received). Runs exactly once when a
launch fails after creation (readiness, registration, or initial-prompt
failure) — even when a `kill` is configured. For script sets without a
configured `kill`, the retained `cleanup` argv also serves as the
final-member teardown fallback. `quecto container gc` also runs it — the
record's retained argv for a `stopped` record, the selected config's for
an unrecorded orphan — so it must remove the container **and** the state
directory, and must succeed when the directory is already gone (the
shipped `kill.sh --op cleanup` then still removes the container it would
have named, `quecto-<environment_id>`).

## The standard container (`quecto container init`) (#2024 S4e)

`standard` is the name of **each repository's own default container**: the
one an agent grabs for this folder and this folder's needs. The bundle the
binary carries is two things with two owners. The official Docker/Podman
adapter scripts (below) are **quecto's**: trusted, language-neutral,
versioned and integrity-checked. The Containerfile is **the project's**:
init writes a neutral starter only where none exists, and from then on the
file is yours to edit and commit — never counted as drift, never replaced,
`--refresh` included. `quecto container
init` materialises the bundle under the repository and binds the repository to it
through the repo-local overlay; nothing is copied from the source tree and
no config is hand-edited. Run it from the **repository root** (or pass
`--project <root>`): the overlay is the working directory's own
`.quecto/config.json`, which an agent started at the root reads only at
the root, so a project below the checkout's toplevel (`git rev-parse
--show-toplevel`) is refused, naming the root to pass. A directory that
is no checkout at all is accepted as it is.

```
quecto container init [--project <abs dir>] [--repo <url>] [--image <tag>] [--refresh] [--dry-run]
quecto container status [--project <abs dir>]
```

**Runbook** (the same the `docs` tool serves as `container-runtime`; every
step is a command with its expected output):

| Step | Command (from the repository root) | Expected |
|---|---|---|
| Preconditions | `git rev-parse --show-toplevel`; `podman --version` (or `docker`); `jq --version`; `git ls-remote --exit-code origin`; `quecto status` | the toplevel is `pwd`; the tools answer; `ls-remote` exits 0; `Overlay: none` or `(trusted)` |
| 1. init | `quecto container init` (`--repo <url>`, `--image <tag>`, `--dry-run`) | `wrote …` per missing file (× 5 in a bare repo; a committed Containerfile is `kept … (this project's own; …)`), `container config "standard" written as container_configs.standard in <repo>/.quecto/config.json (trusted for exactly these bytes)`, `default: true`, then `next:` with the build command |
| 2. Containerfile | make `.quecto/containers/standard/Containerfile` this repo's: the project's toolchain (pinned where the repo pins it) and `LABEL ai.quecto.required-tools="…"`; tools only, never source or credentials; skip only when it already carries them | the file shown and approved; commit it |
| 3. build | the printed `podman build -t quecto-<folder>:local -f <repo>/.quecto/containers/standard/Containerfile <repo>/.quecto/containers/standard` (skip when `status` already reports the image present) | `Successfully tagged localhost/quecto-<folder>:local` |
| 4. verify | `quecto container status` | a header, the `assets/config/trust/image` lines (plus a `this repo's default` continuation, or a `note:` when the label was removed by hand), then `ready: spawn {"container":true} from an agent in this project`, exit 0 (exit 1 `not ready` while a script differs or a file is missing; the project's own Containerfile never counts) |
| 5. doctor | `quecto container doctor` | every check `✓` (`gh` may be `!`), `0 checks failed`, exit 0 |
| 6. spawn | from an agent in the repo: `spawn {"agent_id":"probe","task":"run pwd","container":true}` | `environment_ref=C1 container_config=standard` |
| 7. inventory | `agent_cmd {"agent_id":"*","command":"get_containers"}` | the environment `running`, its `repository` |
| 8. kill | `agent_cmd {"agent_id":"*","command":"kill_container","ref":"C1"}` | the environment gone from `get_containers` |
| Rollback | `quecto config unset --local container_configs.standard`; `rm -r .quecto/containers/standard`; optionally `podman rmi quecto-<folder>:local` | `unset container_configs.standard in <repo>/.quecto/config.json (trusted)` |
| Upgrade | `quecto container init --refresh`, `quecto container doctor` | each differing script `refreshed`; the Containerfile `kept … (this project's own; never replaced — review it before building)` |

Failures and their fixes are in the [troubleshooting runbook](#troubleshooting-runbook)
below; the trust boundary and upgrade rules follow.

What `init` does, in order (every refusal below happens before anything
is written, so a refused init leaves the project untouched):

1. Resolves the repository: `--repo`, else the checkout's `origin` remote
   (`git remote get-url origin`), else none — a **sandbox** entry (empty
   workspace, no clone), which the output names as such. A URL that embeds
   any credential is refused, from either source — the overlay is a
   shareable repository file. Over `http`/`https` **any** userinfo is a
   credential: `https://user:secret@host/…` and GitHub's token form
   `https://ghp_xxx@github.com/…` alike. Over `ssh://`, `git://` and the
   scp form a bare user names an account (`ssh://git@host/…`,
   `git@github.com:org/repo`) and is fine; only `user:password@` there is
   refused.
2. Asks the configuration writer whether it would accept the write, and
   reads the effective container-config set (global file plus the trusted
   overlay). An overlay that exists but is **not trusted**, whatever it
   declares, is refused with the way out (`quecto config trust`), on a
   `--dry-run` too: init never adopts content it did not write. An
   explicit `--config` is refused (it replaces the overlay). The standard
   entry is **always** written with `"default": true` (#2035): a global
   default is overridden in this repo only (`the global default <name>
   does not apply in this repo` — the global file is untouched); another
   overlay entry carrying the label loses it in the same write and the
   output names it (`displaced default: <name>`; select it by name with
   `container: {"mode":"new","container_config":"<name>"}`).
3. Judges every destination under `<project>/.quecto/containers/standard/`
   — `Containerfile`, `scripts/create.sh`, `scripts/exec.sh`,
   `scripts/inspect.sh`, `scripts/kill.sh`: missing, identical to the
   embedded bytes, or differing. A differing **script** is drift (kept and
   reported; a launch refuses it; `--refresh` restores it). A differing
   **Containerfile** is the project's own: reported as `kept … (this
   project's own; never replaced — review it before building)`, by
   `--refresh` too. A symbolic link in a file's place or in
   any directory on the way from the project down (`.quecto`,
   `containers`, `standard`, `scripts`) is refused, not followed; `status`
   and `--dry-run` see the same and report it as `refused`.
4. Writes `container_configs.standard` into `<project>/.quecto/config.json`
   through the configuration writer (`quecto config set --local` path):
   exclusive hold, validated as a layer and as the merge, trust recorded
   for exactly the bytes written. Every argv names the materialised
   scripts by absolute path; `create` carries `--state-dir
   <base dir>/container-environments`, `--repo <url>` when there is one and
   `--image quecto-<folder>:local` (or `--image` as given) — named after
   the project folder so repositories in differently named folders build
   their own images: lowercased, ASCII letters and digits kept, a single
   `.` or `_` between them kept, any other run a `-`, leading and trailing
   runs dropped, at most 100 characters; a folder name with nothing
   admissible gets the adapter's `quecto-dev:local`. Folders that share a
   name share a tag, and **a git worktree is a different folder** (a
   different tag, so another build unless the layer cache is warm): pass
   `--image` to tell same-named repos apart or to make a worktree share the
   main checkout's image. An existing entry keeps the tag it has (one with
   no `--image` keeps the adapter's default); `kill`/`cleanup`
   carry `--op kill`/`--op cleanup`; no argv ends with `--` (the launcher
   appends it before the child command).
5. Materialises the missing files (the scripts byte-identical to
   `scripts/container-runtime/docker/*.sh`, executable), each written
   whole (temporary file, fsync, rename) and **never replaced** unless
   `--refresh` is given: an edited **script** is kept and reported as
   differing from the embedded version — and a launch refuses it (see the
   trust boundary below); `quecto container init --refresh` renames the
   embedded bytes over every differing script and reports each as
   `refreshed`. The Containerfile is the exception in both directions: the
   project's version is kept as its own and `--refresh` never touches it. Should a write still fail here (a
   filesystem race after step 3), the error says the entry was already
   written and how to finish (run init again) or roll back
   (`quecto config unset --local container_configs.standard`).
6. Prints the files, the entry and the one step left — the exact build
   command:
   `podman build -t quecto-<folder>:local -f <project>/.quecto/containers/standard/Containerfile <project>/.quecto/containers/standard`
   (on a docker-only host, the same command with `docker`: the scripts
   drive whichever runtime the doctor's `runtime-cli` line names).

Running `init` twice changes nothing (no files, byte-identical overlay).
A re-init keeps the existing entry's `--repo` and `--image` unless the
flag is given — the origin remote is not re-derived over a `--repo` you
chose — and prints `kept:` / `rewrote:` / `added:` lines for each so nothing changes
silently.

**The trust boundary: host-side scripts.** The overlay's trust record
covers `.quecto/config.json` — the entry and the argv it names — not the
files those argv point at. The scripts under
`.quecto/containers/standard/scripts/` run **on the host**, as the user,
before any container exists (`create.sh` clones the repository and
starts the container; `exec.sh`, `inspect.sh` and `kill.sh` drive it),
so a change to them that arrives with a `git pull` is as consequential as
a change to the overlay itself. What is verified is precisely **the
program — the first argv element — of each argv the host is about to
run**, whenever it lies under a `.quecto/containers/standard/`
directory, compared with the bytes this quecto embeds the way `status`
compares them:

- at **create**, every program the selected entry names (`create`,
  `exec`, `inspect`, `kill`, `cleanup`), before the create runs —
  `spawn container: true` and `quecto container doctor` alike;
- at **join** (`container: {mode: existing}`), the program of the
  environment's *retained* exec argv, before the exec runs;
- at **inspect**, **kill** and **cleanup**, the program of the retained
  argv, before it runs.

The arguments after the program (`--state-dir`, `--repo`, `--image`,
`--op`) are the overlay's, covered by its trust record; a path that
reaches the bundle through `..` is refused outright rather than treated
as a script of the entry's own. A program that differs, is missing, or is
a symbolic link refuses the operation naming the file and the way back:

```
container config 'standard' refused: /repo/.quecto/containers/standard/scripts/create.sh differs from the standard bundle this quecto embeds (or this quecto embeds a newer bundle than the one that wrote it — run `quecto container init --refresh`); it is a host-side script the launch would run before any container exists, so review the change (git diff) and restore the bundle with `quecto container init --refresh` (or delete the file and run `quecto container init`)
```

A refused create or join is a plain tool error and nothing ran. A
refused kill leaves the environment in the retryable `cleanup-failed`
state with the same reason as its last error (`environment C1 cleanup
failed: retained kill refused: … ; state is cleanup-failed, retry
kill_container`); a refused inspect is an inspect failure with the argv
kept for retry; a refused cleanup is logged and skipped. In every case
the altered script did not run: restore the bundle, then retry.

`quecto container status` keeps listing the file as `differs` (and the
image line carries the same refusal). Review a pulled change to
`.quecto/containers/standard/` exactly as you would review one to
`.quecto/config.json` — `git diff` the directory before `init --refresh`.
An entry that names its own scripts elsewhere is vouched for by the
overlay's trust alone: the integrity check is the standard bundle's, not
a general one. The Containerfile is not a host-side script and is not
checked at launch; it is the project's own file, so `status` lists it as
`Containerfile: this project's own` (or `yours` beside a drifted script)
and stays `ready`. Commit `.quecto/containers/standard/Containerfile` so the
next agent in the folder builds the same container. This repository keeps
the scripts and `.quecto/config.json` local (init materialises them); a
repository that commits its scripts gets the integrity check on every pull. Because the file
is the project's, quecto raises no flag over it: **read a cloned
repository's Containerfile before you build it**, as you would any build
script — its `RUN` steps execute in your build, and the image later gets the
mounted checkout and the GitHub token. Init says so on the `kept` line.

**Upgrades.** The comparison is against *this binary's* bundle, so a
quecto that embeds a newer bundle than the one that materialised the
files sees every changed script as `differs` — indistinguishable from an
edit, and refused the same way (the message says so). After upgrading
quecto, run `quecto container init --refresh` in each checkout: it
renames the embedded bytes over every differing script (never the
project's Containerfile), reports each as `refreshed`, and a newer quecto's assets replace an older one's. Review
the diff first if the checkout's copy carries local changes you meant to
keep; environments created before the refresh are torn down by the
refreshed scripts, which is what the check is for.

**Image approval.** The image only has to exist locally under the tag the
entry names: the create's preflight checks `podman image exists` /
`docker image inspect` and the `run` passes `--pull=never` (podman; Docker
CLI ≥ 20.10), so nothing is ever fetched implicitly. An
earlier draft (PR #2020) bound a digest-pinned image to a signed approval
record through four `QUECTO_PODMAN_*` variables; that ritual is not carried
over: it had to be performed by hand for every rebuild, put four
environment variables between an operator and a working container, and
sat *before* the preflight, so `quecto container doctor` could never have
reported on it. Pin a digest in `--image` if you want one.

**Image contents.** The starter is tooling-neutral: a digest-pinned Debian
trixie base with a shell, Git/GitHub, search (`ripgrep`, `fd`), `jq`, Python
and a C toolchain for native dependencies — no language toolchain and no
`ai.quecto.required-tools` label. It ends with a commented section showing
where the project's toolchain and that label go. This repository's own
container is the worked example: `.quecto/containers/standard/Containerfile`
adds pinned Rust, Clippy, rustfmt, LLVM tools, `cargo-nextest`,
`cargo-llvm-cov`, `cargo-deny` and `cargo-machete`, and declares those eight
tools in its label.
`ENTRYPOINT []` lets the create adapter's child argv remain the container's
main process. The host
`quecto` binary is identity-mounted, so the image retains a compatible glibc.
The repository is still cloned by the trusted host adapter and mounted at
runtime; source and credentials are never baked into the image.

**What the doctor asks of an image.** The adapter is tooling-neutral: it
knows no language. Two checks run the image (`run --rm --pull=never`):

- `image-base` — the image has a shell and `git`, which is all the harness
  itself needs inside a container.
- `required-tools` — the image's own promise. An image may declare
  `LABEL ai.quecto.required-tools="python3 uv pytest ruff"` (bare executable
  names separated by whitespace: ASCII letters, digits, `.`, `_`, `+`, `-`;
  at most 64 names of at most 64 characters); the doctor then proves each one
  is on `PATH` and fails **naming the missing tool**, so an image that lost a
  tool is refused before an agent edits code. No label (or an empty one), no
  extra check. A label that is not a list of tool names is refused without
  being run. Quecto's own Containerfile declares its eight Rust tools this way;
  a `--image` of your own is asked only for what *it* declares. A label is
  inherited through `FROM`: an image derived from quecto's own keeps the
  Rust list unless it redeclares the label. An image built before the label
  existed carries none and is asked only for a shell and git — rebuild it
  (`quecto container init --refresh` prints the command) to get the check
  back. It is skipped, and says so, while `image-base` fails.

Both checks run the image as `run --rm <image> sh -c …`, exactly as a create
runs the child: the image needs `ENTRYPOINT []` (as the starter declares) or an entrypoint that executes its arguments. One that ignores them
answers for the probe, and for the agent.

A probe the runtime could not run is reported as the runtime's failure, not
the image's: no answer within `QUECTO_REPO_CHECK_TIMEOUT`, or a `run` that
the runtime itself refused, exits 3 like any other runtime failure; an image
with no usable `sh` exits 6. `quecto container status` reports its `image`
line from the first of these three checks that failed, so it never says
`ready` for an image the doctor refuses.

`quecto container status` reports, one line each and exit 1 while anything
is missing: the assets (`present (5 of 5, version 5)`, or which differ or
are missing), the `standard` entry of the effective set (`default` with a
`this repo's default` line when the overlay declares it labelled; `default
by rule` plus a `note:` with the remedy — `quecto container init --refresh`
or `quecto config set --local container_configs.standard.default true` —
when `quecto config unset --local` removed the label (possible only while
another entry is labelled; a raw edit un-trusts the overlay and status then
reports `withheld`), since launch policy still selects it;
`default`/`not default` by label for a global entry of that name, which is
nobody's standard; its `--repo`), the trust of the overlay
(`trusted`, or `withheld` with the remedy; a destination init would refuse,
such as a symbolic link in a file's place, is listed as `refused` with a
`note:`), and the image as the entry's own
create preflight reports it (`--preflight-only`, the S4b contract — the
same line `quecto container doctor` shows).

Rollback: `quecto config unset --local container_configs.standard`, then
delete `.quecto/containers/standard`.

## The canonical reference runtime

The repository ships one canonical reference runtime — a script set that
implements every operation of the contract above and is executed end to end
by the epic acceptance suite
(`quecto-agentic-harness/tests/features/script_managed_runtime_slice5.feature`)
through the production script adapter and strict parser:

- [`scripts/container-runtime/create.sh`](../scripts/container-runtime/create.sh)
- [`scripts/container-runtime/exec.sh`](../scripts/container-runtime/exec.sh)
- [`scripts/container-runtime/inspect.sh`](../scripts/container-runtime/inspect.sh)
- [`scripts/container-runtime/kill.sh`](../scripts/container-runtime/kill.sh)

### Shared inference admission (#1679)

When the parent runs with an `admission` section, `create` and `exec` receive
`QUECTO_ADMISSION_DIR`: the authority's client directory. A script that can
expose that directory to the child **at the same path** must do so and add
`"admission_capability": "shared-directory-v1"` to its JSON result; the bundled
Docker/Podman adapter bind-mounts the directory and reports the capability
(joins report it only when the environment was created with the same
directory). An admission-enabled parent refuses to launch when the capability
is missing, so a script that cannot expose the directory simply omits the
field and the launch fails before any inference. Nothing else in the contract
changes for parents without admission.

The bundled adapter mounts the directory at its lexical normal form (a doubled
slash, a `/./` or a trailing slash are the same directory) and records the
configured spelling for `exec` to compare. It refuses, before any environment
state exists: a relative path, or one with control characters or a `:` (it is
mounted as `src:dst:mode`); a spelling with a `..` (the child opens the
configured spelling, and only the normal form is mounted); a client directory
that is itself a symbolic link (the read-write mount would expose whatever it
points at); an authority that is `~/.quecto` itself, or that lives outside
`~/.quecto` but inside the socket directory (mounted read-write whole, with
nothing to mask it); and —
when the authority lives inside the identity-mounted `~/.quecto` — a spelling
that is not under `$HOME/.quecto` or reaches the authority through a symbolic
link inside it, because the read-only mask would then miss the real directory.
A symlinked `HOME`, or `~/.quecto` being a symbolic link, is fine: the harness
derives the path from `HOME`, so mask and identity mount agree.


The reference runtime is **host-local**: it needs no Docker and runs
everywhere (including CI). Each script takes `--state-dir <dir>` — a trusted
root under which it keeps one directory per environment (checkout workspace,
recorded child pids, invocation records). The root is created owner-only
(mode 700) and adopted only when owned by the invoking user, and each
environment directory is minted with `mktemp -d` — an unpredictable name
that fails hard rather than reuse (or follow a symlink planted at) an
existing path. `create.sh` clones the repository baked into its own argv
(`--repo <url>`; omit it for a sandbox config with an empty workspace) into
`<state>/<environment_id>/workspace/repo` and starts the child directly on
the host with the checkout (or the sandbox workspace) as its working
directory, so the agent genuinely operates inside its isolated workspace; a
failure after state allocation
(e.g. a failed clone) rolls the partially created environment directory back
so nothing unreachable by `cleanup` is ever leaked. `exec.sh` starts a
joining child in that same checkout;
`inspect.sh` reports whether any recorded child is still alive; `kill.sh`
serves both the `kill` and `cleanup` operations, distinguished by
`--op kill` / `--op cleanup`, and performs trusted-root containment (the
environment directory must resolve under the `--state-dir` root) before any
destructive removal. `kill` keeps the environment's recorded metadata for
the cleanup that follows; `cleanup` is terminal and removes the entire
per-environment directory, so the state root does not grow with every
environment ever created. All scripts fail fast (`set -euo pipefail`), log to
stderr only, encode stdout results with a real JSON encoder (`jq`, which
must be installed), and pass the repository URL as a literal argv element
(never shell-interpolated).

A matching configuration (the `--repo` baked into the create argv makes
this config self-contained; drop it for a sandbox config):

```json
{
  "container_configs": {
    "container-runtime": {
        "default": true,
        "create": ["/repo/scripts/container-runtime/create.sh", "--state-dir", "/var/tmp/quecto-envs", "--repo", "https://github.com/you/project"],
        "exec": ["/repo/scripts/container-runtime/exec.sh", "--state-dir", "/var/tmp/quecto-envs"],
        "inspect": ["/repo/scripts/container-runtime/inspect.sh", "--state-dir", "/var/tmp/quecto-envs"],
        "kill": ["/repo/scripts/container-runtime/kill.sh", "--state-dir", "/var/tmp/quecto-envs", "--op", "kill"],
        "cleanup": ["/repo/scripts/container-runtime/kill.sh", "--state-dir", "/var/tmp/quecto-envs", "--op", "cleanup"]
    }
  }
}
```

The reference runtime reports a direct `socket_path` endpoint. The
`socket_proxy` endpoint form is an authoring option for runtimes whose
sockets are only reachable inside the environment (see "Endpoints and
liveness"); its production behavior — bridge lifecycle, readiness probing,
EOF-pushed death — is exercised by the proxy scenarios in
`quecto-agentic-harness/tests/features/script_managed_liveness_slice3.feature`.
Forwarded nested descendants do not run the top-level endpoint negotiation in
their ancestor session; when such a descendant reports a script-managed direct
socket, ancestors omit `socketPath` and live commands return a clear
non-connectable-socket error instead of exposing the container-local path.

## The official Docker/Podman adapter

Alongside the host-local reference set (which remains the CI-exercised
default), the repository ships an official Docker/Podman adapter implementing
the same contract (the directory keeps its historical `docker` name):

- [`scripts/container-runtime/docker/create.sh`](../scripts/container-runtime/docker/create.sh)
- [`scripts/container-runtime/docker/exec.sh`](../scripts/container-runtime/docker/exec.sh)
- [`scripts/container-runtime/docker/inspect.sh`](../scripts/container-runtime/docker/inspect.sh)
- [`scripts/container-runtime/docker/kill.sh`](../scripts/container-runtime/docker/kill.sh)

Design properties:

- **Rootless Podman by default, Docker as fallback.** Every script resolves
  its CLI to `podman` when present, else `docker`; `QUECTO_CONTAINER_CLI`
  overrides. Rootless Podman is the intended runtime for an autonomous
  spawner: membership of the `docker` group is root-equivalent on the host
  (the daemon runs as root with no policy layer, so anything holding the
  socket can bind-mount `/` and escalate), whereas rootless Podman runs the
  container as the invoking user inside a user namespace. `create.sh` runs
  the child as the host uid/gid and, under Podman, adds `--userns=keep-id`
  so the identity-mounted paths keep their ownership inside the container.
  The `metadata.runtime` field reports whichever CLI was used.
- **A real init at PID 1.** `create.sh` passes `--init` so a minimal init
  (catatonit under Podman, tini under Docker) reaps orphaned grandchildren
  and forwards signals. Without it the child is PID 1 itself: nested agents
  whose parent exited accumulate as zombies, and `stop` hangs until the
  SIGKILL escalation because PID 1 ignores an unhandled SIGTERM. The child
  remains the container's liveness; the init exits when the child does.
- **One container per environment, child as PID 1.** `create.sh` starts the
  child as the container's main process, so Docker's view of the container is
  exactly the child's liveness; `exec.sh` joins later members with
  `docker exec` into the same container.
- **Identity bind-mounts.** The per-environment workspace (rw), the parent's
  socket dir (rw), the child binary (ro), the child's `--config` file (ro,
  when outside `$HOME/.quecto`), and `$HOME/.quecto` (rw) are mounted at the
  same path inside and outside, so the child's CLI args need no rewriting and
  the UDS socket it binds appears directly on the host.
- **`HOME` preserved, `QUECTO_BASE_DIR` never overridden.** `QUECTO_BASE_DIR`
  is quecto's credentials/config home; overriding it inside the container
  detaches the child from the identity-mounted `$HOME/.quecto` and breaks
  OAuth providers. The scripts carry a comment warning against this.
- **Image selection.** `--image <img>` on the create argv, or the
  `QUECTO_DOCKER_IMAGE` environment variable, with a sensible local default
  (`quecto-dev:local`; an entry written by `quecto container init` always
  passes `--image`, by default named after the project folder). The `run` passes `--pull=never`: a tag that vanished
  between the preflight and the run fails instead of fetching whatever a
  registry serves under that name.
- **Pid fence.** `create.sh` passes `--pids-limit` (default `16384`;
  `QUECTO_CONTAINER_PIDS_LIMIT` overrides, `-1` defers to the user slice,
  `0` is refused). Threads count against the container's pid cgroup and the
  runtime default of 2048 is exhausted by an in-container `cargo test`,
  after which every fork fails and the environment dies with all its
  members.
- **Preflight before any state exists (#2024 S4b).** `create.sh` runs one
  list of checks — runtime CLI (`podman`/`docker`, `QUECTO_CONTAINER_CLI`),
  `jq`, `git` (when `--repo` is given), `gh` (a warning only: without it
  members get no GitHub token), the image present in the local store
  (`podman image exists` / `docker image inspect`; **never an implicit
  pull** — the `run` passes `--pull=never`, which needs Docker CLI ≥ 20.10), `--repo` reachable (`git ls-remote`, bounded by
  `QUECTO_REPO_CHECK_TIMEOUT`, default 15 s, no credential prompt), and the
  state dir writable and owned by the current user (or, when it does not
  exist yet, creatable under a writable parent) — before the environment
  directory is created. A normal create dies at the first
  failure with a message that names the remedy and a distinct exit code
  (2 usage, 3 no runtime, 4 no jq, 5 no git, 6 image missing, 7 `--repo`
  unreachable, 8 state dir); `--preflight-only` evaluates every check and
  prints the tab-separated report `quecto container doctor` presents,
  leaving no trace (not even the state dir). A runtime that cannot answer
  the image lookup (daemon down, socket permission, `QUECTO_REPO_CHECK_TIMEOUT`
  exceeded) is its own failure with exit 3, never "image missing"; an
  empty repository (no `HEAD`) is a warning, since the clone accepts it.
  The host-local reference `create.sh` implements the same mode with its
  own subset (jq, git, `--repo`, state dir).
- **Rollback and containment.** `create.sh` installs an ERR trap that removes
  partial state and `docker rm -f`s any container it managed to start; every
  destructive operation proves the environment id contains no path
  separators and resolves under the trusted `--state-dir` root, mirroring the
  host-local set. `kill.sh` serves `--op kill` / `--op cleanup` and logs each
  operation to the state root. Under Podman it bounds the remove's grace to
  one second (`--time 1`): Docker's `rm -f` kills immediately, Podman's would
  otherwise wait the container's ten-second stop timeout, which does not fit
  the parent's signal-driven exit budget.
- **Strict JSON contract.** All stdout results are emitted with `jq`, exactly
  matching the `create`/`exec`/`inspect` wire contracts above. `create.sh`
  reports `metadata.checkout` (the members' working directory, which is the
  swarm checkout root) so the supervising session can keep a swarm's
  environment when its coordinator is lost (#1924).
- **Member harness logs reach journald.** `create.sh` passes
  `-e RUST_LOG=${RUST_LOG:-info}`; without it the member harness's
  redacting subscriber is a no-op and an environment that dies leaves no
  trace of why (termination signal, teardown, socket close). Set `RUST_LOG`
  on the host before spawning to change the level for that environment. The
  variable reaches the member harness only: the harness scrubs it (with the
  `QUECTO_SWARM_*` identity) from every tool child it starts, so a member's
  `cargo test` or other Rust programs keep their own logging defaults.
  Under rootless Podman with its default `journald` log driver the
  container's stdout/stderr land in the user journal, so read a member's
  logs with:

  ```sh
  journalctl --user CONTAINER_NAME=quecto-env-<id>
  ```

  where `env-<id>` is the environment id from `get_containers` (the
  container is named `quecto-<environment_id>`). Add `-f` to follow, or
  `--since`/`--until` around the time an environment was lost. Under Docker
  (default `json-file` driver) read them with `docker logs quecto-env-<id>`
  instead; a Podman configured with another driver likewise uses
  `podman logs`. `kill.sh`
  additionally logs every kill/cleanup it performs to `kill.log` in the
  state root, which is the first place to look when an environment vanished.
- **Host-side clone is transport-restricted.** The repo URL from the config's
  own `--repo` argv is cloned on the host before any container exists, so
  `create.sh` runs `git clone` under `GIT_ALLOW_PROTOCOL=file:https:ssh:git` —
  command-running transports (`ext::…`) can never execute host commands
  (PR #1401 review; config files travel, so the restriction stays).
- **Provider API keys never enter the docker-side container config.**
  `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` / `OPENROUTER_API_KEY` /
  `FIREWORKS_API_KEY` are written to a `0600` file in the `0700` state dir,
  identity-mounted read-only, and sourced by a bootstrap `sh` that `exec`s
  the child (which therefore still ends up as PID 1). Passing them with
  `docker run -e` would persist them in the container config, readable via
  `docker inspect` for the container's whole lifetime (PR #1401 review).
  Requires `/bin/sh` in the image.
- **GitHub access works inside the environment.** Agent workflows need `gh`
  and git-over-https pushes, and a host keyring is unreachable from a
  container. `create.sh` resolves the token host-side (`gh auth token`) and
  ships it as `GH_TOKEN`/`GITHUB_TOKEN` through the same `0600` secret file;
  git identity (global gitconfig only, for determinism) and the
  `gh auth git-credential` helper travel as non-secret `GIT_CONFIG_*`
  environment entries, so the host gitconfig — which may carry LFS filters or
  keyring helpers the image lacks — is never mounted. `exec.sh` gives joiners
  the identical contract (including sourcing the secret file). Requires `gh`
  in the image for API/push use; everything else degrades gracefully when no
  token is available.

A matching configuration:

```json
{
  "container_configs": {
    "docker": {
        "default": true,
        "create": ["/repo/scripts/container-runtime/docker/create.sh", "--state-dir", "/var/tmp/quecto-docker-envs", "--repo", "https://github.com/you/project"],
        "exec": ["/repo/scripts/container-runtime/docker/exec.sh", "--state-dir", "/var/tmp/quecto-docker-envs"],
        "inspect": ["/repo/scripts/container-runtime/docker/inspect.sh", "--state-dir", "/var/tmp/quecto-docker-envs"],
        "kill": ["/repo/scripts/container-runtime/docker/kill.sh", "--state-dir", "/var/tmp/quecto-docker-envs", "--op", "kill"],
        "cleanup": ["/repo/scripts/container-runtime/docker/kill.sh", "--state-dir", "/var/tmp/quecto-docker-envs", "--op", "cleanup"]
    }
  }
}
```

CI has no container runtime, so the Docker/Podman adapter is not exercised by
the CI BDD lanes; it is verified manually against local rootless Podman, and its
shape (existence, fail-fast mode, contract needles, cross-links) is pinned
by `quecto-agentic-harness/tests/docs/container_runtime_docs.rs`.

## Troubleshooting (runbook)

Every container failure now carries the script's own words; the first
move is always the same: read the tool error (or the harness stderr),
then run the doctor from the directory the agent runs in and apply the
remedy it prints.

```
quecto container doctor                 # the effective default config of this directory
quecto container doctor --name <config> # a named entry
quecto container doctor --config <file> # that file's entries, ignoring the overlay
```

The doctor resolves the container config exactly as `spawn container:
true` does (global file plus the directory's trusted `.quecto/config.json`
overlay; an untrusted overlay that declares `container_configs` is
reported and withheld), runs the create script's `--preflight-only` mode
**without creating an environment**, prints one line per check
(`✓` passed, `!` warning, `✗` failed) with a `remedy:` line under each
warning or failure, and exits 1 when any check failed:

```
container config "quecto" (create: /…/docker/create.sh --state-dir /var/tmp/envs --repo https://github.com/you/project)
  ✓ runtime-cli     podman at /usr/bin/podman
  ✓ jq              jq at /usr/bin/jq
  ✓ git             git at /usr/bin/git
  ✓ gh              gh at /usr/bin/gh
  ✗ image           image quecto-dev:local is not present in the local podman store
    remedy: build it (podman build -t quecto-dev:local <dir with its Containerfile>) or pull it (podman pull quecto-dev:local); create never pulls implicitly
  ✗ image-base      image quecto-dev:local could not be checked for a shell and git
    remedy: fix the image check first
  ✗ required-tools  the tools image quecto-dev:local declares were not checked
    remedy: fix the image-base check first
  ✓ repo            --repo https://github.com/you/project is reachable
  ✓ state-dir       state dir /var/tmp/envs is writable and owned by the current user
3 checks failed, 0 warnings
```

| Symptom (tool error / stderr) | Doctor line | Remedy |
| --- | --- | --- |
| `script-managed create failed with status exit status: 3: … no container runtime on PATH` | `✗ runtime-cli` | Install rootless Podman (preferred) or Docker and put it on PATH, or set `QUECTO_CONTAINER_CLI`. |
| `… status exit status: 4: … jq is not on PATH` | `✗ jq` | Install `jq`. |
| `… status exit status: 5: … git is not on PATH` | `✗ git` | Install `git` (only needed for configs with `--repo`). |
| `… gh is not on PATH: members will have no GitHub token` (a warning; the create proceeds) | `! gh` | Install `gh` and run `gh auth login` so members can push and use the GitHub API. |
| `… status exit status: 6: … image quecto-dev:local is not present` | `✗ image` | Build the image (`podman build -t quecto-dev:local <dir>`) or pull it; the scripts never pull implicitly. Change `--image` / `QUECTO_DOCKER_IMAGE` if another image was meant. |
| `… status exit status: 6: … image <img> cannot host an agent: missing git` | `✗ image-base` | The image needs a shell and `git`; add them to its Containerfile and rebuild. |
| `… status exit status: 6: … image <img> does not provide a tool it declares in ai.quecto.required-tools: missing <tool>` | `✗ required-tools` | Rebuild the image from its Containerfile, or correct the `ai.quecto.required-tools` label. A label that is `not a tool name` must list bare executable names separated by spaces. |
| `… status exit status: 3: … could not read the ai.quecto.required-tools label of image <img>` / `… could not run image <img>` / `… did not answer within 15s while running image <img>` | `✗ required-tools` / `✗ image-base` | The runtime failed, not the image: check the daemon/service (`podman info`); `QUECTO_REPO_CHECK_TIMEOUT` raises the bound. |
| `… status exit status: 7: … --repo <url> is unreachable: fatal: …` | `✗ repo` | Check the URL and your credentials (`ssh` key or `gh auth login`); fix `--repo` in the config's create argv. |
| `… status exit status: 8: … state dir <dir> is not owned by the current user` / `cannot be created` | `✗ state-dir` | Point `--state-dir` at a directory you own. |
| `unknown container config '<name>' (available container configs: …)` | (doctor refuses too) | Pick a listed name, or bind the repository: `quecto config set --local container_configs.<name> '{…,"default":true}'`. |
| `container: true refused: the checkout's repo-local config overlay was not applied` | `container doctor refused: …` (exit 1, before any check) | Review `.quecto/config.json`, then `quecto config trust` in that directory; or `--name` a global entry. |
| `… status exit status: 3: … podman could not look up image …` / `did not answer within 15s` | `✗ image` (runtime cause quoted) | The runtime is installed but cannot answer: start the daemon/service (`podman info`, `systemctl --user start podman.socket`, docker group membership). |
| `container config '<name>': create script … does not support --preflight-only` | — | The config's create script predates the preflight contract (both shipped script sets implement it); add the mode to a custom script (see "Script contract"). |
| `container config '<name>': create script … exited exit status: 2 after 4 checks without reporting a failure: … QUECTO_REPO_CHECK_TIMEOUT must be a positive integer` | — (exit 1) | The script died mid-report; its last words follow the colon. Fix what they name (here the environment variable) and rerun. |
| `container config '<name>': create script … printed a malformed check line …` / `… without reporting the mandatory check …` | — (exit 1) | A custom create script's `--preflight-only` output is not the contract (see "Script contract"); the doctor never presents a partial report as healthy. |
| `… status exit status: 1: …` after the preflight (clone, `podman run`) | all `✓` | Read the quoted stderr: the clone or the runtime refused. The create rolled its environment back; `kill.log` in the state dir lists every kill/cleanup. |
| `script-managed exec failed with status …: …` | — | The join's own stderr is quoted; the environment may have exited — `agent_cmd get_containers` shows its status. |
| `container config 'standard' refused: …/scripts/create.sh differs from the standard bundle this quecto embeds …` (spawn, doctor, a retained kill → `cleanup-failed`) | — (exit 1, before any check) | A script under `.quecto/containers/standard/` was edited, pulled, or written by an older quecto: `git diff .quecto/containers/standard`, then `quecto container init --refresh`; retry. |
| init: `… is not the repository root (<root>): … run init from the root, or pass --project <root>` | — | `cd` to the checkout's toplevel, or pass `--project <root>`. |
| init: `… carries a credential in its userinfo …` | — | `git remote set-url origin <credential-free url>` or `quecto container init --repo <url>`; a helper, ssh key or `gh auth login` supplies the credential at clone time. |
| init: `overlay … is not trusted (sha256 …); review it and run \`quecto config trust\` first` | — | Review `quecto config get --local`, `quecto config trust`, run init again (init never adopts content it did not write). |

Eight create-then-rollback cycles in one afternoon with nothing but
`status 1` to show for them is what this runbook replaces (#2024).

## How to author another runtime adapter

To adapt the reference runtime to Docker, Podman, devcontainers, or any
other isolation mechanism, copy `scripts/container-runtime/` and replace
only the marked `--- Runtime-specific section ---` in each script; the argv
parsing, environment-variable handling, and JSON results stay identical.
Rules an author must keep:

1. **Structured argv, no shell interpolation.** Every configured operation
   is an argv array executed directly; treat your own `--repo` value and the
   child command after `--` as opaque literals.
2. **Start the child exactly once.** `create`/`exec` own the child's whole
   lifetime inside the environment; Quecto never starts a fallback child.
3. **Exactly one JSON object on stdout**, produced by a real JSON encoder,
   with exactly the documented fields (`environment_id`, `workspace_path`,
   `metadata`, and exactly one of `socket_path`/`socket_proxy` for `create`;
   `metadata` plus one endpoint for `exec`; `metadata` plus optional
   `status` for `inspect`). Logs go to stderr only.
4. **Honor the identity split.** `create` receives the session ref in
   `QUECTO_CONTAINER_ENVIRONMENT_REF`; every later operation receives the
   runtime-owned id you reported, in `QUECTO_CONTAINER_ENVIRONMENT_ID`.
5. **Trusted-root containment before destructive cleanup.** Never remove a
   path you have not proven to resolve under your own trusted state root.
6. **Keep runtime knowledge in the scripts.** Quecto's Rust code contains no
   Docker/Podman/devcontainer special cases and must never need any.

## Planned inference-admission transport (#1679)

[ADR-0026](../quecto-agentic-harness/docs/architecture-design-records/adr-0026-shared-inference-admission.md)
fixes the private admission transport contract. **Not implemented or enabled by
this documentation:** existing container scripts do not yet provide admission.
The planned official same-host Docker/Podman adapter uses a dedicated private
admission socket-directory mount; create, join and nested launches must verify
reachability to the same authority. Custom runtimes without path visibility need
an explicit child-to-host reverse bridge capability. Existing `socket_proxy`
connects the parent to the child and is not proof of this reverse capability.
Never expose the entire agent-control socket directory as an admission endpoint.

The P0 prototype in
`quecto-agentic-harness/tests/integration/inference_admission_transport.rs` uses real local
processes and a test-only stdio bridge, not Docker or production admission.
Actual supported-runtime create/join/nested, cancellation and restart evidence is
required in P3 before activation. Unsupported enabled transport must fail closed,
not silently substitute an in-process or container-local budget. No multi-host
coordination is promised.
