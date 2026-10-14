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

`show(task, version)` gives that version of the task's plan as `plan`, whole and
in the same shape, whatever its state, a draft included (#2481 M4b), in place of
the accepted plan; everything else in the answer is the same. A version the task
does not have is refused (`unknown_plan_version`); one whose file is set aside is
refused as `artifact_set_aside`; and one the board does not hold while a kept file
in `plans/` whose name spells no plan is set aside (it may hold that version) is
refused as `set_aside_file`.

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
never an offset it counted itself. With `version`, they page through that
version instead. Given either, `show` answers only the task's id (`task`),
`may_be_incomplete`, and `plan`: the plan's `version` and `updated`, with the
approach from `approach_from` (as `approach_from` and `approach`) and the
task items from `items_from` (as `items_from` and `items`), whichever were
named; a part not named is left out. With no plan to page through, `plan` is
`null`. The agent tool cuts a page as it cuts a whole `show`, with
`approach_more`, `approach_next`, `items_more` and `items_next`, so an agent
reads on with `show(task, approach_from: approach_next, items_from: items_next)`
until no `_next` is given. Every page moves on: where no cut keeps a character
as read whole, a page cuts inside it rather than read nothing. Every page names
the plan's `version` and `updated`; if either differs from the first answer's,
the plan changed while it was read (a draft edited between two pages, say), so
read again from 0.

## Proposing and editing a plan

A plan says how a task will be done: its approach (markdown), its task items
(`id`, `title`, `criteria`, `depends_on`) and its team, in the shapes `show`
gives them (#2481 M4b). Each version is its own file, `plans/<task>.<n>.md`, and
a change to a plan writes that file alone: the task's file is left as it is.

- `propose_plan(task, approach, items, team)` adds a draft, numbered after the
  task's last version, proposed by the committer. `approach` and `items` are
  required (`""` and `[]` are allowed); `team` is optional, and `null` or
  leaving it out means no team.
- `show(task, version)` reads a draft whole before it is built on. An agent that
  proposes from a plan it read must read all of its approach and task items
  first, following `approach_next` with `approach_from` and `items_next` with
  `items_from` until no `_next` is given: an approach or a list of items the
  tool's 64 KiB cap cut must never be written back as if whole.
- `edit_plan(task, version, approach, items, team, expected_updated)` edits a
  draft as a patch. A field left out keeps what the draft holds, byte for byte.
  A field given replaces the draft's whole: `items` is the whole list of task
  items, `team` the whole team (`null` takes it away), and `approach` the whole
  approach. So an agent changes the task items without sending the approach
  again, which may be longer than one `show` answer gives. An edit names at
  least one of `approach`, `items` and `team`. With `expected_updated`, the
  draft's `updated` as it was read, a draft changed since is refused
  (`plan_changed`), never edited over. Without it, a write that loses a race to
  another writer applies the same patch to the draft as that writer left it.

Anyone may propose a plan or edit a draft. Both are refused for a task that is
done or archived (`task_finished`), while the task's own file is set aside
(`set_aside`), and while a file named as one of the task's plans is set aside
(`artifact_set_aside`), since the next version's number may be that file's. A
set-aside file in `plans/` whose name spells no plan holds neither up: its name
can never be a version's, so no write lands on it, where `show(task, version)`
refuses a version it cannot find while such a file is set aside
(`set_aside_file`), since that file may hold it. An edit is refused for a version
the task does not have (`unknown_plan_version`), one that is not a draft
(`plan_not_draft`), and one that changes nothing (`no_edit`). Text is normalised
as it enters, as a task's is: the approach as markdown, and each task item's title
and criteria as one line. A number out of its range is refused naming the field as
the request spells it (a role's `members` outside 1 to 32 is
`team/roles/0/members`). A plan the board's own schema refuses, with what the
draft keeps, is the rule `invalid_plan`, naming the plan's field: `text` for text
past the most a plan holds (320 KiB, counted as it enters the board). Both ops
answer `plan`: its `task`, `version`, `state`, `proposed_by`, `edited_by`,
`created` and `updated`. They do not echo its text. The CLI takes `--items` and
`--team` as JSON.

## Accepting or discarding a plan

A draft becomes the task's plan by `accept_plan(task, version, expected_updated)`
(#2481 M4c). One commit writes the task, naming the version (`accepted_plan`),
the plan, `accepted` with who accepted it and when, and the version accepted
before, now `superseded`. All three share one stamp. An older draft accepted
over a newer accepted version is a rollback: the newer one is superseded. A
superseded plan is never accepted again; propose a new version instead.
`expected_updated` is required: the `updated` that `show(task, version)` gave,
so what is accepted is what was read. A draft changed since is refused
(`plan_changed`), also when an edit lands while the acceptance is being
written. The answer gives the `task`, the `plan` and the version it
`superseded` (`null` when none was accepted).

`discard_plan(task, version, expected_updated)` sets a draft aside for good: it
is kept, as `discarded`, and only it is written. `expected_updated` is optional,
as for `edit_plan`. The answer gives the `plan`.

On a task someone holds, only the holder of a live claim accepts or discards
(`not_holder`, or `claim_expired` for the holder: renew first); on a task
nobody holds, anyone does. Both are refused for a done or archived task
(`task_finished`), for a version that is not a draft (`plan_not_draft`) or that
the task does not have (`unknown_plan_version`), and while the task's file or a
file named as one of its plans is set aside. An acceptance is also refused
while a run of the task is going (`run_open`) or a file named as one of its
runs is set aside (it may be the open one), and while the task's plans or runs
contradict each other (`plans_inconsistent`, `runs_inconsistent`): mend them
first.

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
| `op` | The op the request named: `bootstrap`, `list`, `list_ready`, `list_takeable`, `show`, `create`, `update_draft`, `move`, `renew`, `take_over`, `link_pr`, `unlink_pr`, `complete`, `clear_set_aside`, `propose_plan`, `edit_plan`, `accept_plan`, `discard_plan`; `unknown` for a request that names none, or none the board has. A request the agent tool does not take (`bootstrap`) records its op, refused, and a `quecto board` line refused as it is read records the op it named before the refusal |
| `front_end` | `tool` (the agent `board` tool) or `cli` (`quecto board`) |
| `client` | `agent` for the tool; for the CLI `cli`, or as `QUECTO_BOARD_CLIENT` names it: `pocket_relay` or `other` |
| `access` | `read` or `write`: what the op does, even when it was refused before it was read |
| `outcome` | `ok` or `refused` |
| `code`, `rule` | For a refusal, the answer's error code (`invalid_request`, `rule`, `not_found`, `stale`, `contended`, `clock_ahead`, ...) and, for `rule`, the rule's code (`not_holder`, `claim_expired`, `not_allowed`, ...). `internal` is no answer's code: an agent tool call whose work ended without an answer (it panicked) is recorded as it unwinds, refused as `internal`. A `quecto board` panic aborts the process and writes no record |
| `field`, `fault`, `unexpected_count` | For `invalid_request`, `invalid_task` and the rules `invalid` and `invalid_plan`, which field was wrong: a schema field as the board's table spells it (`depends_on/2` is `depends_on`, `items/3/criteria/0` is `items`), `op`, `request` (the request as a whole; a plan's text past the most a plan holds, `invalid_plan`'s `text`, is recorded as `request`, `too_large`), or `other`; and how: `missing`, `unexpected`, `wrong_type`, `invalid_value`, `too_large`, `not_json`, `duplicate_key`. A key no schema of the op names is `other`, `unexpected`, and `unexpected_count` counts such keys, never naming one, even when another op takes a field of that name |
| `task`, `pr` | The task the op's `task` names, and the pull request `link_pr` or `unlink_pr` names, once the request was read |
| `head` | The board's head an answer named, only when it is a full commit id git produced |
| `listed`, `shown`, `truncated_more`, `set_aside`, `set_aside_more` | For an answered listing: the tasks it listed, those its answer showed and those it left out, and the set-aside files it listed and left out. The agent tool's listing is cut to 32 KiB; the CLI leaves nothing out |
| `plans`, `runs` | For an answered `show`: how many versions of its plan and how many runs the task holds, as the board read them; counts only, never their text |
| `plan_version`, `items`, `roles` | For `propose_plan`, the version it wrote, only once written. For `edit_plan`, `accept_plan` and `discard_plan`, the version it named, also when the board refused the change. For `show(task, version)`, the version it named once the task was read, also when that version was refused; a `show` of a task the board refused records none. A request refused as it is read records none. For `propose_plan` and `edit_plan`, once written, how many task items and team roles the plan holds (`roles` is 0 with no team); counts only |
| `plan_state_from`, `plan_state_to` | For `accept_plan` and `discard_plan`: the state of the version it named as the change last read it (`draft`, `accepted`, `superseded` or `discarded`; none when the task has no such version, or the read was refused first), and the state it wrote (`accepted` or `discarded`; none for a refused change) |
| `status_from`, `status_to` | For a write of a task (`create`, `update_draft`, `move`, `renew`, `take_over`, `link_pr`, `unlink_pr`, `complete`): the task's status as the write last read it (none for `create`), and the status it moved to: a `move`'s target, made or refused, otherwise the status of the task it wrote (`create`'s `draft` or `ready`, `complete`'s `done`), so a refused write other than `move` writes no `status_to`. A `link_pr` or move to `review` that completes its task records its own write's status here; what came of completing it is in `completion` |
| `claim` | For a `move`, `renew`, `take_over`, `link_pr` or `unlink_pr` of a task that holds a claim, the claim as the change found it, never its holder: `held_s` (seconds since it began, below zero when the writer's clock reads before that), `left_s` (seconds until it expires, below zero once it has lapsed) and `holder_is_writer` |
| `attempts`, `lost`, `pause_ms` | For a write (every op that writes but `bootstrap`): the attempts it began (at most 3), how many of them lost the race to another writer, and, when it paused between attempts, the time it paused in milliseconds. A `link_pr` or `move` that completes its task counts only its own write's attempts here; the completion's write is counted in `completion_attempts` |
| `completion`, `completion_code`, `completion_rule`, `prs_linked`, `prs_pending` | After a `link_pr` or a move to `review` of a task that links pull requests: `done` (every one merged), `pending` (some not merged yet, counted in `prs_pending`) or `failed` (the host, the store or a rule failed it, with its error code in `completion_code` and, for a rule, the rule's code in `completion_rule`); `prs_linked` counts the pull requests the task links. `complete` records only `done` or `pending`: a `complete` refused for any other reason records no completion, and its `code` says why |
| `completion_attempts`, `completion_lost`, `completion_pause_ms` | For a completion run after a `link_pr` or a move to `review`, its own write's attempts, lost races and pause, as `attempts`, `lost` and `pause_ms` count the change's |
| `normalised` | For `create`, `update_draft`, `propose_plan` and `edit_plan`, what the board's normaliser changed in each text field the request gave (`title`, `description`; a plan's `approach`, and its `items`: every kind of change made to any task item's title or criteria; a `create` that gave no description, or an empty one, tells none, nor does an `edit_plan` that gave neither an approach nor task items): a list of `crlf`, `lone_cr`, `line_break`, `chat_space`, `tab`, `edge_blank_lines`, `trailing_space`, `edge_space`, empty when it kept the field as written; never the text |
| `cancelled` | Written `true` only when the tool call was dropped (its turn interrupted) while its work ran: the work still records it. A call dropped before its work started reached no board and writes no record |
| `build_us` | How long building the board's operations took, when the request got that far |
| `result_bytes` | The size of the answer as its front end sent it |
| `duration_us` | The whole call, from its arrival to its answer |

Kinds are added, never renamed or reused, so a reader of the log must accept a
kind or a field it does not know.
