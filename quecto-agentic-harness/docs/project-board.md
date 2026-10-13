# Project board

A repository's project board (#2477, epic #2481) is the shared list of tasks
people and agents plan, claim and finish. Its tasks live on the repository's
`quecto/board` branch, and are written only through the board's two front ends:
the agent `board` tool, offered to a top-level agent, and the `quecto board` CLI,
which scripts and the pocket relay run. Both read the same JSON request and give
the same JSON answer; see the README's tool table and `quecto board --help`.

## The board branch

The branch's root holds a `README.md`, a `format/board` marker, and one folder
per kind of artifact: `tasks/<id>.md`, `plans/<task>.<n>.md`, `runs/<task>.<n>.md`,
`changes/<task>.<n>.md`, `reviews/<task>.<change>.md` and `discussions/<id>.md`,
each its kind's canonical markdown. An `archive/` tree is room left for archived
work: it is never read, and every write carries it as it is. A file that does not
read as what its name spells is set aside, never trusted.

This build reads and writes every folder. A write writes each artifact as its
kind's file and carries every other file by its object. A folder the write leaves
alone is carried as the same tree. A folder it writes in, or one holding a stray
(any entry a write drops), is rebuilt from the files it carries, and a folder left
with nothing in it is omitted. A file set aside is carried as it is, and writes to
its key are refused, unless it is one a write drops: a write of its key replaces
it. A clear removes one file set aside, in any folder. While a name outside `tasks/` is listed more than once, every
write and clear is refused (`folder_set_aside`) until a later quecto can clear it.
Listings are unavailable (`folder_set_aside`) while a file set aside outside
`tasks/` names a plan, run, change, review or discussion, since the index holds
tasks alone for now. `show` reads the store itself, so it still answers then (see
below). It writes the marker `quecto-board 3` and reads `quecto-board 2` too. A
build that reads only format 2 asks for an upgrade on a board this build has
written, rather than taking its folders for someone else's branch.

A read takes at most the following. Past any of these it answers
`board_too_large` before it reads any file's bytes. In a partial clone, objects the
clone lacks are still fetched before their sizes can be judged.

| Folder | Entries | File bytes | One file |
|---|---|---|---|
| `tasks/` | 4,096 | 16 MiB | 512 KiB |
| `plans/` | 4,096 | 24 MiB | 1.5 MiB |
| `runs/` | 4,096 | 16 MiB | 768 KiB |
| `changes/` | 4,096 | 4 MiB | 8 KiB |
| `reviews/` | 4,096 | 16 MiB | 1 MiB |
| `discussions/` | 2,048 | 16 MiB | 1 MiB |
| **The board** | **16,384** | **64 MiB** | |

A write keeps every folder and the board within these bounds too, so it never
makes a board a read refuses, except that it leaves at most 2,048 entries in
`tasks/`; the 4,096 a read takes leave room for a board written before task files
were markdown (a JSON file and a summary per task). A file larger than its kind's
cap is set aside, not read.

## Show

`show(task)` reads the task, its plans and its runs from one read of the board
(#2481 M4a). Besides `head` and the whole `task`, its answer holds:

| Field | Meaning |
|---|---|
| `plan` | The task's accepted plan, whole: the version the task names, when the board holds it accepted. `null` while it has none, or while its file is set aside |
| `plans` | Each version of the task's plan, by version, in brief: `version`, `state`, `proposed_by`, `updated` |
| `runs` | Each readable run of the task, by number: the latest readable one whole (its task items, owners and evidence), every other in brief (`number`, `plan_version`, `state`, `started`, `ended`, `cost_tokens`). A run's owners and evidence have credentials redacted, as text from outside the board does |
| `may_be_incomplete` | Whether a file that may hold one of the task's plans or runs is set aside: one of the task's own, or a kept file whose name spells none in `plans/` or `runs/`. Its plans and runs may then be missing one |
| `quarantined` | Those files, as a listing's `quarantined` shows them; one of a plan or run also names its `kind`, `task` and `number` |

The CLI gives the answer whole. The agent tool gives an answer within 64 KiB as
it is, and cuts a longer one to fit. The task is never cut, so a task whose own
description is long can take more than 64 KiB alone; what does not fit beside it
is then counted, not shown. A cut answer keeps, in order:

1. the accepted plan whole if it fits. Otherwise it says `plan_truncated: true`
   and keeps the plan's other fields, as much of its approach as leaves room
   for what follows (at least half the room is the approach's, cut where it
   keeps characters as read, such as accented letters, emoji sequences and
   flags, whole where it can), and the leading task items that fit. It
   counts what it left out (`approach_more` characters, `items_more` items) and
   says where to read on (`approach_next`, `items_next`). `plan: null` with
   `plan_truncated` means the plan was cut whole: not even its other fields fit
   beside the task;
2. the newest runs: the latest whole, or with its leading task items
   (`latest_run_items_more` left out), then older runs in brief (`runs_more`
   left out); no run when the latest does not fit;
3. the newest plan versions (`plans_more` left out);
4. the files set aside (`quarantined_more` left out).

A plan at the board's limits, with every run and version, is about 1.6 MB whole.

### Reading on through a plan

`show` takes two optional fields that page through the task's accepted plan:
`approach_from`, a character of its approach, and `items_from`, the index of one
of its task items, each from 0 (`null` is the same as leaving it out). A
character here is a Unicode scalar value, not what a reader sees as one, so an
agent passes back only the `approach_next` and `items_next` an answer gave,
never an offset it counted itself. Given either, `show` answers only the task's id
(`task`), `may_be_incomplete`, and `plan`: the accepted plan's `version` with
the approach from `approach_from` (as `approach_from` and `approach`) and the
task items from `items_from` (as `items_from` and `items`), whichever were
named; a part not named is left out. With no plan to page through, `plan` is
`null`. The agent tool cuts a page as it cuts a whole `show`, with
`approach_more`, `approach_next`, `items_more` and `items_next`, so an agent
reads on with `show(task, approach_from: approach_next, items_from: items_next)`
until no `_next` is given. Every page moves on: where no cut keeps a character
as read whole, a page cuts inside it rather than read nothing. Every page names
the accepted plan's `version`; if it differs from the first answer's, the plan
changed while it was read, so read again from 0.

## Telemetry

Each board call, from either front end, writes one `board_op` record (epic #2481
T3), only while the event log is on (`"telemetry": {"event_log": {"enabled":
true}}`, off by default). With it off no `board_op` record is built, timed or
written, to the event log or to `tracing`. The board clock's one warning when
it has no outside time (`quecto::project_board`) is still written whatever the
switch says; gating it is telemetry slice T5's.

- **The agent tool** records in its session's event log,
  `<base_dir>/audit/<session>.jsonl`, under the session's own rule: the loaded
  configuration or your global config switches it on, and a repository overlay
  may switch it on, never off, and the switch is decided when the session
  starts. Its records follow a session switch to the arriving session's log,
  with that switch. A session that keeps no event log from its start (`-s -`,
  `--no-session`) records nothing; a switch into a session that persists no
  log keeps recording into the departing session's log, as the agent's own
  events do.
- **`quecto board`** loads no layered configuration, so it follows your global
  config (`<base_dir>/config.json`) alone. While that switches the event log on,
  it writes the day's log, `<base_dir>/audit/board-cli_<UTC date>.jsonl`, opened
  by its first record, so a call that records nothing leaves no file. The daily
  logs are kept with no retention yet. Each stops at the event log's 256 MiB
  cap, which is only approximate here: each process counts from the file's size
  when it opened it, so processes writing at once can overshoot it.

A record is appended on the call's own thread, under the log's write gate,
filed under no turn. It never holds up the board: it waits for the gate at most
50 ms, and past that it is dropped and counted, and the next record written is
preceded by one `{"event":"board_ops_dropped","dropped":N}` line. A record the
log cannot take is dropped with one warning, and never changes the call's
answer. The drop count lives with the recording, so it can go unnoted: a
`quecto board` process writes one record, so a drop of its one record is
never noted, and a session switch starts a new count for the arriving
session's log, losing the departing session's unnoted drops. The same record is also written to `tracing` (target
`quecto::project_board`, message `board op`: `DEBUG` for a read, `INFO` for a
write), only while the event log is on.

**A record holds no board text.** Its fields are kinds the code spells (an op of
the board's table, an error or rule code, a schema field), counts, a task's
slug (redacted and capped at 64 characters), a pull request's number and a
commit id git produced. Never a title, a description, a message, a problem or
reason, a path, a name, an email, a remote, a token, nor any key or value a
request sent. The session's event log still holds, as for every tool, the
generic `tool_call` and `tool_result` records of each `board` tool call, with
its arguments and its answer's preview word for word (the README's "tool
arguments and result previews as sent"); an audit view of the board tool's
arguments (slice T0) awaits the owner's decision. Lifecycle questions (time in
each status, claim lifetimes) are answered from the board branch's own history,
not from these records.

A caller names itself in `QUECTO_BOARD_CLIENT`, read by allowlist:
`pocket-relay` is recorded as `pocket_relay`, any other value as `other`, never
as given. It is for the pocket relay to set; an older quecto ignores it.

| Field | Meaning |
|---|---|
| `session` | As on every event-log record: the agent's session for the tool; `board-cli:<UTC date>` for `quecto board` |
| `turn` | Always `null`: a board call is filed under no agent turn |
| `op` | The op the request named: `bootstrap`, `list`, `list_ready`, `list_takeable`, `show`, `create`, `update_draft`, `move`, `renew`, `take_over`, `link_pr`, `unlink_pr`, `complete`, `clear_set_aside`; `unknown` for a request that names none, or none the board has. A request the agent tool does not take (`bootstrap`) records its op, refused, and a `quecto board` line refused as it is read records the op it named before the refusal |
| `front_end` | `tool` (the agent `board` tool) or `cli` (`quecto board`) |
| `client` | `agent` for the tool; for the CLI `cli`, or as `QUECTO_BOARD_CLIENT` names it: `pocket_relay` or `other` |
| `access` | `read` or `write`: what the op does, even when it was refused before it was read |
| `outcome` | `ok` or `refused` |
| `code`, `rule` | For a refusal, the answer's error code (`invalid_request`, `rule`, `not_found`, `stale`, `contended`, `clock_ahead`, ...) and, for `rule`, the rule's code (`not_holder`, `claim_expired`, `not_allowed`, ...). `internal` is no answer's code: an agent tool call whose work ended without an answer (it panicked) is recorded as it unwinds, refused as `internal`. A `quecto board` panic aborts the process and writes no record |
| `field`, `fault`, `unexpected_count` | For `invalid_request`, `invalid_task` and the rule `invalid`, which field was wrong: a schema field as the board's table spells it (`depends_on/2` is `depends_on`), `op`, `request` (the request as a whole), or `other`; and how: `missing`, `unexpected`, `wrong_type`, `invalid_value`, `too_large`, `not_json`, `duplicate_key`. A key no schema of the op names is `other`, `unexpected`, and `unexpected_count` counts such keys, never naming one, even when another op takes a field of that name |
| `task`, `pr` | The task the op's `task` names, and the pull request `link_pr` or `unlink_pr` names, once the request was read |
| `head` | The board's head an answer named, only when it is a full commit id git produced |
| `listed`, `shown`, `truncated_more`, `set_aside`, `set_aside_more` | For an answered listing: the tasks it listed, those its answer showed and those it left out, and the set-aside files it listed and left out. The agent tool's listing is cut to 32 KiB; the CLI leaves nothing out |
| `plans`, `runs` | For an answered `show`: how many versions of its plan and how many runs the task holds, as the board read them; counts only, never their text |
| `status_from`, `status_to` | For a write of a task (`create`, `update_draft`, `move`, `renew`, `take_over`, `link_pr`, `unlink_pr`, `complete`): the task's status as the write last read it (none for `create`), and the status it moved to: a `move`'s target, made or refused, otherwise the status of the task it wrote (`create`'s `draft` or `ready`, `complete`'s `done`), so a refused write other than `move` writes no `status_to`. A `link_pr` or move to `review` that completes its task records its own write's status here; what came of completing it is in `completion` |
| `claim` | For a `move`, `renew`, `take_over`, `link_pr` or `unlink_pr` of a task that holds a claim, the claim as the change found it, never its holder: `held_s` (seconds since it began, below zero when the writer's clock reads before that), `left_s` (seconds until it expires, below zero once it has lapsed) and `holder_is_writer` |
| `attempts`, `lost`, `pause_ms` | For a write (every op that writes but `bootstrap`): the attempts it began (at most 3), how many of them lost the race to another writer, and, when it paused between attempts, the time it paused in milliseconds. A `link_pr` or `move` that completes its task counts only its own write's attempts here; the completion's write is counted in `completion_attempts` |
| `completion`, `completion_code`, `completion_rule`, `prs_linked`, `prs_pending` | After a `link_pr` or a move to `review` of a task that links pull requests: `done` (every one merged), `pending` (some not merged yet, counted in `prs_pending`) or `failed` (the host, the store or a rule failed it, with its error code in `completion_code` and, for a rule, the rule's code in `completion_rule`); `prs_linked` counts the pull requests the task links. `complete` records only `done` or `pending`: a `complete` refused for any other reason records no completion, and its `code` says why |
| `completion_attempts`, `completion_lost`, `completion_pause_ms` | For a completion run after a `link_pr` or a move to `review`, its own write's attempts, lost races and pause, as `attempts`, `lost` and `pause_ms` count the change's |
| `normalised` | For `create` and `update_draft`, what the board's normaliser changed in each text field the request gave (`title`, `description`; a `create` that gave no description, or an empty one, tells none): a list of `crlf`, `lone_cr`, `line_break`, `chat_space`, `tab`, `edge_blank_lines`, `trailing_space`, `edge_space`, empty when it kept the field as written; never the text |
| `cancelled` | Written `true` only when the tool call was dropped (its turn interrupted) while its work ran: the work still records it. A call dropped before its work started reached no board and writes no record |
| `build_us` | How long building the board's operations took, when the request got that far |
| `result_bytes` | The size of the answer as its front end sent it |
| `duration_us` | The whole call, from its arrival to its answer |

Kinds are added, never renamed or reused, so a reader of the log must accept a
kind or a field it does not know.

### Fold-back records

A started task's run, once its swarm has ended, is folded back onto the board
(epic #2481 S4): its outcome, where each of its plan's task items stood, owners,
evidence and cost, written to the run alone in one commit. Each fold-back
writes one `board_fold_back` record under the same switch, gate and rules
(written to `tracing` at `INFO` as `board fold back`). It holds no titles,
owners or evidence, only kinds and counts:

| Field | Meaning |
|---|---|
| `task`, `run` | The task (its slug, redacted and capped as above) and the run's number |
| `outcome`, `code`, `rule` | `ok`, or `refused` with the error and rule codes a `board_op` would carry. A run that had already ended (folded back, or ended as lost by the merge that completed its task) is `ok` with only `attempts` and `lost` |
| `run_outcome` | How the run ended: `succeeded`; `failed` (the swarm failed or exhausted its budget); `stopped` (blocked or cancelled) |
| `items` | The run's task items by state: `todo`, `doing`, `review`, `done`, `blocked` |
| `unmatched`, `extra` | Plan items no swarm item matched by its title (`<slug>: <title>`) or, failing that, by its `<slug>: ` prefix, left `todo`; swarm items no plan item took (of several under one key, all but the most progressed, then newest), not kept |
| `evidence`, `evidence_dropped` | Evidence lines kept, and those left out: refused by the board's schema or file format, repeated, or past an item's 16 or the run's 256 KiB (owners counted first). A run that would leave the board, or its `runs/`, past the bytes a read takes is written again without any, every line counted dropped; any other invalid write is refused, with nothing left out |
| `cost_tokens` | The tokens the swarm was observed to spend |
| `attempts`, `lost`, `bytes` | The writes' attempts and lost races (both passes, when the evidence was left out), on every outcome; and the bytes of the run's text |
| `duration_us` | The whole fold-back |
