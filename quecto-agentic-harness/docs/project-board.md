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
it. A clear removes one file set aside, in any folder.

A name listed more than once (a hand edit; git rejects such a tree) is resolved
only by a clear, which names, for every such name on the board, the copy to keep or
that none is; quecto never picks a copy. While one is listed in `tasks/`, every
write is refused. While one is listed in another folder, a write into that folder
is refused, and any other write carries the folder as the tree it was, so no write
makes a tree holding a name twice. A clear that keeps a copy is refused when the
copy would take a folder or the board past what a read takes: keep a smaller copy,
or drop every copy. A clear resolving thousands of names, near its caps (8,192
names, 16,384 copies), may not fit in one 1 MiB tool request; the refusal's other
way out, moving `quecto/board` back to the commit before the hand edit, covers it.

Listings are unavailable (`folder_set_aside`) while a file set aside outside
`tasks/` names a plan, run, change, review or discussion, since the index holds
tasks alone for now. `show` reads the store itself, so it still answers then (see
below). Every refusal over a file set aside (`set_aside`, `artifact_set_aside`,
`set_aside_file`, `folder_set_aside`) names the file's `path` and `object` and,
in its message, the way out: mend the file, or discard it with
`clear_set_aside(path, object)`, which discards the file's content.

`quecto board --remote <name>` naming a remote the clone does not have answers
`invalid_request` on field `remote`. The agent tool names no remote (its board is
`origin`), so there a missing `origin` answers `store_unavailable`, as git failing
to say whether a remote exists does.

`quecto board --board-id <16 hex digits> --remote-url <url>` names where the
board is, for a caller that already knows (the pocket relay): the id its records
name it by and the remote's fetch URL. A read takes each as given, so git is not
asked for it; with both, a read is composed with one git process, the check that
`--remote` is a remote the clone configures (the request itself then reads the
board with git). A remote counts only when git resolves it to a URL that is
neither empty nor the name itself, so a folder of that name is never read (a
remote configured with its own name as a relative path, such as `git remote add
backup backup`, is refused for every request for the same reason: give it a
URL git cannot take for the name). A write takes neither flag on trust: git
names the location. A board id that is not 16 lower-case hex digits, or a URL
with a control or invisible character or over 4096 bytes, answers
`invalid_request` on field `board-id` or `remote-url`.

`quecto board --url <url>` sends every request to a board server instead
(epic #2705 B7): `unix:///<socket path>`, or `http://<ip>:<port>` on loopback or
a Tailscale address with its token in `QUECTO_BOARD_TOKEN` (or `board.token` in
the global config), never on the command line. Without `--url`,
`QUECTO_BOARD_URL` or the global config's `board.url` names one, for the agent
tool too. The server answers each request exactly as this machine's board would,
and a write is one round trip, made as `QUECTO_BOARD_COMMITTER` when it is set,
else as the clone's committer. A `start` is begun there and its coordinator
launched here (B8). A URL of another shape, or TCP without a token, answers
`store_unavailable`; so does a
server that cannot be reached after 3 tries, or that refuses the request (its
status and code in the `reason`). A server that stays busy for 10 s answers
`busy`: try again shortly. See `docs/board-server.md`.

The board writes the marker `quecto-board 3` and reads `quecto-board 2` too. A
build that reads only format 2 asks for an upgrade on a board this build has
written, rather than taking its folders for someone else's branch.

### A board moved to a board server

When a board moves to a board server (epic #2705), a one-time fence commit on
`quecto/board` raises the format to 4: `format/board` reads `quecto-board 4`, and
beside it `format/MOVED` names the board and the URLs that serve it now. The
README is written as every write writes it, every folder and the archive stay as
they were, and strays are dropped as by any write. `MOVED` is
UTF-8 text of LF-ended lines, at most 4 KiB:

```
board_id 0123456789abcdef
url https://board.example:7443
url http://100.64.0.2:7443
```

The first line is `board_id` and the board's id (16 lowercase hex digits). Then
come 1 to 8 distinct `url` lines, in the order a client tries them. A URL is
`http://` or `https://`, a host (DNS labels, a dotted IPv4 address or a bracketed
IPv6 address, with no userinfo), an optional port from 1 to 65535 and an optional
path, at most 256 bytes. A build that reads format 3 refuses a format-4 board for reads and
writes alike. This build answers every read and write of a fenced board with
`board_moved`, which gives the `board` id and the `urls`. It never writes to the
branch again. A write from a head before the fence is refused before any commit
is made, judged by the fence's marker alone. A client that has not fetched since
the fence loses its push instead, and its next read answers `board_moved`. A fence whose `format` tree is anything but the marker and a
`MOVED` of this grammar answers `board_moved_malformed` with its `reason`, and is
not read or written either. Building the fence's tree is idempotent: fencing a
board already fenced to the same board and URLs changes nothing. Fencing it to
another place is refused with `board_moved`.

A read takes at most the following. Past any of these it answers
`board_too_large` before it reads any file's bytes.

In a partial clone whose promisor is the board's remote, the first read fetches
the board's commits alone, then its root, then each folder with every file but
those past 1.5 MiB (`--filter=blob:limit`), and the archive's tree alone. Files at
the root (the README, strays) are fetched by their ids, and git sends a file asked
for by id at any size. A file the filter left on the remote is past every cap, so
it is set aside as too large and every write carries it by its id, unfetched.

The clone remembers, in `.git/quecto/project-board/fetch-<remote>`, the last board
it read whole and the files left on the remote. Each new head is then one fetch of
what changed since, as in a full clone, archive changes included (the archive is
never fetched whole, but what a new head adds to it comes with the head). A folder
holding a file left on the remote is never fetched whole again for it. These
fetches keep the clone's own fetch filter. A server that refuses filters is fetched
from whole. A clone whose remote is not its promisor fetches what it lacks whole,
as before.

git skips a head it already holds, so when the user's own `git fetch` (or an
editor's) moved the board first, that one fetch brings nothing: each folder that
changed since the last read is then fetched again whole, and no other folder. That
brings every file in the folder under 1.5 MiB, whether a read reads it or sets it
aside, so at worst about 4,096 files of 1.5 MiB for one folder, far past the 24 MiB
a read takes of `plans/`. Each such fetch leaves a pack holding
copies of objects the clone already has; git's own maintenance (`git maintenance
run`, `git gc`) folds them away, and quecto runs none itself.

The board store needs git 2.44 or later (`GIT_NO_LAZY_FETCH`).

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

Writers in one clone share its tracking ref, `refs/remotes/<remote>/quecto/board`,
and git locks it while one of them fetches. A fetch that finds it locked by
another writer, or moved by one while it held it, is a race this clone lost, never
`store_unavailable`: it fetches again after a short pause, for about 1.5 s in all.
The write's own push is then judged as any other, and a race it lost is counted
in `attempts` and `races_lost`; each fetch again is counted in its `lock_races`.

The board orders its changes by revision (#2705 B9): a stamp is for display.
Every change stamps what it writes with the writer's clock, or the artifact's own
`updated` if that clock reads earlier, and never before its task's `updated`; no
stamp is pushed past the clock, so two changes in one second share one. A claim
runs from the caller's clock for two hours, never from a stamp; a task created
more than 60 s ahead of the caller's clock is not claimed, taken over, renewed or
started (`task_clock_ahead`, naming `created` and `until`, when the caller's
clock reaches the bound): a clock is wrong, or wait until then. A plan's change (an acceptance too) is stamped no earlier than any
plan of its task, and a change's no earlier than any change of its task, each
within the outside time's bound (one stamped past it, by a hand edit or a clock
that ran ahead, is left out). So, kind by kind, a task's own stamps never run
backwards, and neither do its plans' or its changes'; a write of the task itself
(a move, a renewal) is stamped from the task alone, and may read earlier than a
plan or change written just before it.

A caller pins a change to what it read with `expected_rev`, the `rev` that `show`
answers (the board's revision it read at: the git store's commit, a board
server's `<epoch>:<revision>`). The pin holds while the artifact it pins is as the
board held it at that revision, whatever else the board took since; a rev the
board never held never holds. `expected_updated`, the artifact's stamp, is still
taken for one release: its answer carries a `deprecated` note, and a request gives
one pin or the other, never both. It no longer guards as it did: two changes in one
second share a stamp, so a change pinned by `expected_updated` may overwrite one
made in the same second as the read. Move every caller to `expected_rev`.

Board text (a title, a line of prose such as a description or a plan's approach,
and a text cell such as a task item's title or criterion) holds every backslash as
itself: one a reader would take for an escape (before ASCII punctuation, or at the
end, outside code) is written doubled, and read back as one.

## Task descriptions in sections

A task's description is written for people in four fixed sections (#2705 T2),
in this order, one blank line between them:

```markdown
## Goal
Land L12 (#2652): move the UDS presenters, mapping
and loop controllers into interface/uds roles.

## Why
Part of the layout epic #2357.

## Done when
- [ ] Current master merged in, moves only
- [ ] CI green, merged with a merge commit

## Links
- [PR #2652](https://github.com/platform-q-ai/quecto/pull/2652)
- [Planning handoff](https://example.com/handoff)
```

`create` and `update_draft` take the sections as fields: `goal` and `why`
(text), `done_when` and `links` (lists of lines; on the command line, JSON:
`--done-when '["Merged"]'`). The board composes the text above from them. A
request gives either the sections or `description` text, never both. On
`update_draft` the sections replace the whole description: give every section
the task keeps. Agent briefs (scope rules, gate commands) go in the plan or a
handoff, never in the description.

The rules are one table (`ENTRY_RULES` and `PARTS` in
`domain/project_board/entities/task_parts.rs`), so a rule change is a data
edit:

| Rule | On entry |
|---|---|
| Goal | Required: 1 to 3 sentences, one paragraph, no heading or blank line. Sentences are counted at a word ending in `.`, `!` or `?` before a capital, but never at an abbreviation (`Mr.`, `Mrs.`, `Ms.`, `Dr.`, `St.`, `Jr.`, `e.g.`, `i.e.`, `etc.`, `vs.`, `cf.`, `Inc.`, `Ltd.`, `Co.`), an initial (`X.`) or a word with a `.` inside (`U.S.`, `p.m.`): the count may fall short, and over-counts only at an abbreviation the table does not list. Its text must be prose a board file holds (no raw HTML, comments, character references, images, `http://` links or indented lines), kept as written and never rewritten: a bare `https://` or `http://` address is refused with "write the address as [text](https://…)" (one in `code` is kept) |
| Why | Optional: one short paragraph, no heading, prose a board file holds (as the goal's) |
| Done when | Required: 1 to 8 outcomes, each `- [ ] ` (or `- [x] `) and the outcome on one line, text a board file's table cell holds (plain words, `code`, `[text](https://…)` links) |
| Links | Optional: each `- [text](https://…)`, shown as chips. Given as a field, a bare `https://` address is written `[address](https://…)` |
| Description | At most 1,200 characters, headings included: a longer one is refused with "move details into the plan or a handoff" |
| Title | At most 70 characters (Unicode scalar values, not graphemes), written as an outcome |

An empty description still enters, for a draft not written yet. Reading never
checks these rules: a description in any other shape is legacy text, read,
shown and listed as it is and never rewritten (editing a legacy draft's other
fields keeps it byte for byte), and a title reads back to 256 characters.

Every task answer carries the sections beside `description`: `goal` and `why`
(text or `null`), `done_when` (`[{"outcome", "done"}]`), `links` (lines), and
`needs_goal`, `true` for legacy or empty text, which answers no sections. A
listing's `summary` carries the goal on one line as its `subtitle`.

Each field given as a section is checked on its own field before it is
composed: a goal or why with a blank line or a heading, and an outcome or
link that is empty or more than one line, are refused on `goal`, `why`,
`done_when/{i}` or `links/{i}`. Sections given only empty (`links: []`, `goal: ""`)
never clear a description: they are refused on `goal`. Only an explicit
`description: ""` clears one.

A task keeps the form its description entered in, and the form, never the
text's shape, picks its file's layout. A description entered as sections is
written at `schema=2`, a file section each: `## Goal`, `## Why` and
`## Links` as prose, and `## Done when` as a `| Done | Outcome |` table (`yes`
or `no`). Any other description (legacy text, text from the CLI's legacy
entry, or none) stays `schema=1` with its `## Description`, the bytes earlier
builds wrote, however its text reads; an existing task stays at schema 1 on
every write until its description is given anew as sections. A file holding
both layouts, sections at schema 1, or sections that make no description, is
refused.

**Deploy order (strict).** Upgrade every reader first: every host that runs
quecto, the Pocket relay's host included. Only then let agents write sectioned
tasks. A quecto from before T2 reads only schema 1 and never misreads a
schema-2 task; it sets it aside:

- its `list` leaves the task out of `tasks` and names its file under
  `quarantined`, with the reason "schema 2, written by a newer quecto; this
  quecto reads schema 1: upgrade quecto to read it";
- its `show` of the task refuses it as set aside: "task <id> is set aside as
  unreadable; mend its file, or discard it with clear_set_aside(path ...,
  object ...)".

That advice is wrong for a schema-2 task: the file is sound, only the reader
is old. **Never `clear_set_aside` a schema-2 task from an old build** (it
deletes a newer task); upgrade quecto there instead.

### Entry policies by front end

Each front end lets in what its policy allows, an allowlist resolved per
request:

| Front end | Policy | Entry |
|---|---|---|
| The agent `board` tool | Sections only | Every rule above. Free-text `description` is refused (`invalid_task`, field `description`): "is free text: give it as goal, why, done_when and links (move details into the plan or a handoff)" |
| `quecto board` (people, the Pocket relay) | Sections or legacy | A request that gives `goal`, `why`, `done_when` or `links` keeps every rule above. One that gives only `description` text and a title enters as before T2: normalised board text within the schema's limits (65,536 characters, a 256-character title), held as legacy text with `needs_goal: true` |

## List

`list`, `list_ready` and `list_takeable` answer the board's `head`, the
`tasks` they list, by id, and every file set aside as `quarantined`. A
listing reads the board's local index of tasks. Refreshing the index asks the
remote for the board's head (an `ls-remote`), and reads the board whole only
when the board moved since the index was built.

### Summaries (`quecto board ... --summary`)

On the command line, `--summary` (no value; `list`, `list_ready` and
`list_takeable` only, with field flags or `--input` alike) gives each listed task
a `summary` of where its plan and latest run stand. It is the pocket relay's;
the agent tool takes no such field, so its listings, and their pages, are as they
were. Without `--summary` a listing reads and answers exactly what it did before.
Given to any other op it is refused (`invalid_request`, field `summary`) before
anything is built.

```json
"summary": {
  "subtitle": "Land L12: move the UDS presenters." | null,
  "plan": null | { "version": 2, "state": "draft" | "accepted" },
  "run": null | {
    "number": 2,
    "plan_version": 1,
    "state": "running" | "succeeded" | "failed" | "stopped" | "lost",
    "started": "2026-10-10T09:00:00Z",
    "ended": "2026-10-10T09:30:00Z" | null,
    "deadline": "2026-10-11T09:00:00Z" | null,
    "items": { "total": 5, "todo": 1, "doing": 2, "review": 0, "done": 2, "blocked": 0 }
  },
  "items": null | { "total": 5, "not_started": 0, "todo": 1, "doing": 2, "review": 0, "done": 2, "blocked": 0 },
  "may_be_incomplete": false,
  "inconsistent": []
}
```

| Field | Meaning |
|---|---|
| `subtitle` | The task's goal on one line (its paragraph's lines joined by a space); `null` for legacy text |
| `plan` | The newest version of the task's plan that is a draft or accepted (superseded and discarded versions are skipped), so a draft newer than the accepted plan is the one named. `null` when there is none |
| `run` | The task's latest readable run: `number`, `plan_version`, `state`, `started` and `ended` as `show` gives them; `deadline`; and `items`, how many of its task items stand in each state (`todo`, `doing`, `review`, `done`, `blocked`) and in all. `null` when it has none |
| `items` of a running run | All `0`: a run records its task items on the board when it ends (folded back from its swarm, or ended by hand), so while it runs the board holds none. Its swarm's own board has them live |
| `deadline` | Nominal: the run's `started` plus the `deadline_seconds` of the team of the plan the run ran, or a day when the team names none (or the plan names no team), the deadline a start briefs the swarm with. The swarm's own deadline starts a little later, when its coordinator creates it. `null` when it is unknown: the plan the run ran is not readable on the board |
| `items` (top level) | The task items of `plan`, counted by the state `show`'s `item_states` gives each (#2705 T5): `not_started`, or as the latest run recorded it. `null` when there is no `plan`. Page and chat read item states from this one source |
| `may_be_incomplete` | As `show` gives it: a file that may hold one of the task's plans or runs is set aside |
| `inconsistent` | As `show` gives it (#2705 T5) |

Summaries cost a whole read of the board (a fetch), made after the listing,
and are taken only from the board the listing was read at, so a summary always
matches its task's row. When the board moved in between, the listing is read
again: up to 3 reads for the summaries; each listing read again also rebuilds
the index (itself a whole read) when the board moved.

A listing is never refused for its summaries. When they cannot be read, the
listing is answered without them: no task carries a `summary`, and the answer
says why in a top-level `summary_skipped`:

| `summary_skipped` | When |
|---|---|
| `contended` | The board kept moving past the third read (as it does while a swarm runs), or it moved and the listing could not be read again |
| `index_disagrees` | The board stands where the listing was read, yet holds a listed task otherwise than the index lists it (a broken or edited index). Answered at once, not read again |
| the read's code | The whole read failed, with the code it would have been refused with, such as `store_unavailable` |

A task whose plans or runs the board holds twice goes without its own summary
alone: it carries `"summary": null` and its own `"summary_skipped":
"duplicate_artifact"`, and every other task is summarised. Only a listing's own
failure refuses it, as before.

## Show

`show(task)` reads the task, its plans and its runs from one read of the board
(#2481 M4a). Besides `head` and the whole `task`, its answer holds:

| Field | Meaning |
|---|---|
| `plan` | The task's accepted plan, whole: the version the task names, when the board holds it accepted. `null` while it has none, or while its file is set aside |
| `item_states` | Each task item of `plan`, in its order, as `{"id", "state"}` (#2705 T5): the state the task's latest run recorded for it (`todo`, `doing`, `review`, `done`, `blocked`; `todo` while that run is still going, as it records its items when it ends), or `not_started` while the task has no run or its latest run ran another version of the plan. `null` with no `plan`. Kept beside the plan, never in it, so the plan's items go back to `edit_plan` as `show` gives them |
| `plans` | Each version of the task's plan, by version, in brief: `version`, `state`, `proposed_by`, `updated` |
| `runs` | Each readable run of the task, by number: the latest readable one whole (its task items, owners and evidence), every other in brief (`number`, `plan_version`, `state`, `started`, `ended`, `cost_tokens`). A run's owners and evidence have credentials redacted, as text from outside the board does |
| `may_be_incomplete` | Whether a file that may hold one of the task's plans or runs is set aside: one of the task's own, or a kept file whose name spells none in `plans/` or `runs/`. Its plans and runs may then be missing one |
| `quarantined` | Those files, as a listing's `quarantined` shows them; one of a plan or run also names its `kind`, `task` and `number` |
| `inconsistent` | The task's states that contradict its runs and plan (#2705 T5), legacy states the lifecycle never makes: `done_without_plan` (done and completed with no plan accepted and no run, such as a swarm spawned by hand; a task done without a run says so in its resolution instead), `in_progress_without_run` (in progress with no run going), `run_going_outside_progress` (a run going while the task is in any other status). `[]` when there is none, and always `[]` while `may_be_incomplete` (the runs read may not be all of them). A read names them and never rewrites them |

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
4. the files set aside (`quarantined_more` left out);
5. the leading `item_states` that fit (`item_states_more` left out): the
   latest run, kept before them, shows its own items' states already.

A plan at the board's limits, with every run and version, is about 1.6 MB whole.

### Reading on through a plan

`show` takes optional fields that page through the task's accepted plan:
`approach_from`, a character of its approach, `handoff_from`, a character of
its handoff (see "The handoff"), and `items_from`, the index of one of its task
items, each from 0 (`null` is the same as leaving it out). A
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
the board's `rev`; pass the first page's as `expected_rev` on each later page
(#2705 B9). A page so pinned is refused (`plan_changed`) once its plan changed
since the first page (a draft edited between two pages, say): read again from 0.
A change elsewhere on the board does not restart it. Only a page takes the pin.

## Proposing and editing a plan

A plan says how a task will be done: its approach (markdown), its task items
(`id`, `title`, `criteria`, `depends_on`) and its team, in the shapes `show`
gives them (#2481 M4b). Each version is its own file, `plans/<task>.<n>.md`, and
a change to a plan writes that file alone: the task's file is left as it is.

A team is `{"roles": [{"name", "model", "effort", "members", "turns"}],
"deadline_seconds", "image"}`. `deadline_seconds` is how long the run has, in
seconds from Start, 600 to 604800 (ten minutes to seven days), and a day when
it is null or left out; it is a duration, so a plan accepted long before it
starts never starts past its deadline. A team sets no token budget: a run
started from a plan has none (owner decision, 2026-10-14), and `budget_tokens`,
like the old absolute `deadline`, is refused as an unknown key. A plan file
written before then may still hold a `Budget tokens` or `Deadline` row: it is
read past, so the plan reads with no budget and the default deadline, and no
write holds either again. A running board run's deadline is extended as any
swarm's is: `agent_cmd` `swarm_control` `extend` with `deadline_seconds` on its
coordinator.

- `propose_plan(task, approach, handoff, items, team)` adds a draft, numbered
  after the task's last version, proposed by the committer. `approach` and
  `items` are required (`""` and `[]` are allowed); `handoff` and `team` are
  optional, and `null` or leaving `team` out means no team.
- `show(task, version)` reads a draft whole before it is built on. An agent that
  proposes from a plan it read must read all of its approach, handoff and task
  items first, following `approach_next` with `approach_from`, `handoff_next`
  with `handoff_from` and `items_next` with `items_from` until no `_next` is
  given: text or a list of items the tool's 64 KiB cap cut must never be
  written back as if whole.
- `edit_plan(task, version, approach, handoff, items, team, expected_rev)`
  edits a draft as a patch. A field left out keeps what the draft holds, byte
  for byte. A field given replaces the draft's whole: `items` is the whole list
  of task items, `team` the whole team (`null` takes it away), `approach` the
  whole approach, and `handoff` the whole handoff (`""` takes it away). So an
  agent changes the task items without sending the approach again, which may
  be longer than one `show` answer gives. An edit names at least one of
  `approach`, `handoff`, `items` and `team`. With `expected_rev`, the
  `rev` the draft was read at, a draft changed since is refused
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
(`plan_not_draft`), and one that changes nothing (`no_edit`). A proposal or an
edit whose plan, as written, has a `coordinator` role whose model does not name
its provider as `provider/model` is refused with the code `start` gives such a
model (`brief`, code `coordinator_model`), so the planner learns it before the
task is started; an edit is judged on the team it writes, given or kept. Propose
and edit judge the model of every `coordinator` role, whatever its members;
`start` judges the team in its own order, so a coordinator role of more than one
member is refused there first (`team_unsupported`). A plan with no team, or
no `coordinator` role, lets the coordinator be chosen. Text is normalised
as it enters, as a task's is: the approach as markdown, and each task item's title
and criteria as one line. A number out of its range is refused naming the field as
the request spells it (a role's `members` outside 1 to 32 is
`team/roles/0/members`). A plan the board's own schema refuses, with what the
draft keeps, is the rule `invalid_plan`, naming the plan's field: `text` for text
past the most a plan holds (320 KiB, counted as it enters the board). Both ops
answer `plan`: its `task`, `version`, `state`, `proposed_by`, `edited_by`,
`created` and `updated`. They do not echo its text. The CLI takes `--items` and
`--team` as JSON.

### The handoff

A plan's approach is its outline; its handoff holds the detail a coordinator is
briefed with: scope rules, gate commands, what to read first (#2705 T3). The
handoff is markdown, up to 65,536 characters, and optional: none is no handoff,
and a handoff given empty (`""`) is none.

The entry rules are one table (`PLAN_ENTRY_RULES` in
`domain/project_board/entities/plan.rs`), as a task's are:

| Rule | On entry |
|---|---|
| Approach | At most 600 characters when it enters capped; a longer one is refused (`invalid_plan`, field `approach`) with "is N characters; the most is 600: keep the approach to an outline and move the detail into handoff" |
| Handoff | At most 65,536 characters, however it enters |
| Task item title | One line of at most 70 characters, stating the outcome, when it enters capped; a longer one is refused (field `items/{i}/title`) with "is N characters; the most is 70: state the outcome and move the detail into handoff" |
| Task item criteria | 1 to 4 when they enter capped (field `items/{i}/criteria`): none is refused with "has 0 criteria; it takes 1 to 4: give each check the step must pass", more than 4 with "has N criteria; it takes 1 to 4: move the detail into handoff" |
| Task item criterion | One line of at most 150 characters when it enters capped; a longer one is refused (field `items/{i}/criteria/{j}`) with "is N characters; the most is 150: move the detail into handoff" |

A plan read back keeps the schema's own limits for its task items (a line of
500 characters, 16 criteria), so a plan stored before these rules reads and
starts unchanged.

Each front end resolves the entry of the approach and task items from its
policy, as it resolves a task description's:

| Front end | Entry |
|---|---|
| The agent `board` tool | Always capped |
| `quecto board` (people, the Pocket relay) | Capped when the request gives a `handoff` that holds something once normalised (`""` is none, and caps nothing); otherwise as before T3: the schema's limits (an approach of 65,536 characters, task item lines of 500, 16 criteria) |

Only text that enters is checked: an edit that leaves the approach as the
draft holds it (an items-only edit of a plan written before T3, say) keeps it,
however long. Each task item the draft holds exempts at most one given task
item with the same title and criteria (at any index, its id or `depends_on`
changed or not), matched in the order the items are given, so that text is
kept however long or many its criteria. Every other given task item is
checked: a new one, one whose title or a criterion changed, and any further
copy of a held item's text. Reading never checks the caps.

`show` gives `handoff` after `approach`, and its length in characters as
`handoff_chars` (the whole handoff's, even when the agent tool cuts it), only
when the plan has one, so a plan without one answers exactly as before. The agent tool cuts a long handoff as it
cuts the approach, after it: `handoff_more` characters left out, read on with
`show(task, handoff_from: handoff_next)`. A page given `handoff_from` answers
`handoff_from` and the handoff from that character (`null` for a plan with
none).

A started task's coordinator brief carries the handoff verbatim as the data's
`handoff`, before its run, with one fixed sentence pointing to it; a plan
without one briefs exactly as before. The handoff is counted in the brief's
64 KiB, as the brief's data spells it. One past the room the rest of the brief
leaves is cut to fit and ended with "… (cut here; the rest is in the plan on
the board)"; where not even that marker fits, the handoff is left out of the
brief (`handoff_cut`), so a plan that starts without a handoff starts with one
too. The brief already says the full plan is on the board and tells the
coordinator to ask its parent for any part cut.

The handoff, never its text's shape, picks the plan file's layout. A plan with
a handoff is written at `schema=2`, with a `## Handoff` prose section after
`## Approach`. A plan without one stays at `schema=1`, the bytes earlier builds
wrote, however its approach reads (a line that looks like `## Handoff` is
written `\## Handoff`, as prose): every write that gives no handoff (an edit,
an acceptance, a supersession) keeps a plan written before T3 at schema 1, byte
for byte. A schema-1 file holding a Handoff section, or a schema-2 file without
one, is refused. The file's line cap rose from 70,000 to 140,000, so an
approach and a handoff at their most both fit.

**Deploy order (strict).** Every quecto that reads the board (every Pocket
relay host, every laptop, eccup) must be upgraded **before any agent writes a
handoff**. A quecto from before T3 reads only schema-1 plans, and one plan with
a handoff on the board breaks it:

- its `list`, `list_ready` and `list_takeable` fail for the **whole board**,
  not just that task, with `folder_set_aside` on `plans/…`, whose message
  offers `clear_set_aside` and never says to upgrade. The Pocket relay sends
  `list`, so an old relay shows no board at all;
- its `show(task)` still answers, with `may_be_incomplete: true`; the reason
  "schema 2, written by a newer quecto; this quecto reads schema 1: upgrade
  quecto to read it" appears only in that answer's `quarantined[].reason`;
- its `show(task, version)` of the schema-2 version, and `propose_plan`,
  `edit_plan`, `accept_plan` and `discard_plan` on that task, are refused with
  `artifact_set_aside`;
- writes to other tasks still work.

Those refusals offer `clear_set_aside`. **Never follow that advice for a
schema-2 plan**: it deletes the plan, handoff and all, from a board whose file
is sound. Upgrade quecto on that host instead. (Deploy every host together;
there is no two-step release.)

Upgraded, the Pocket relay is unaffected: it sends `show` and `accept_plan` for
plans, reads a plan's keys by allowlist (so `handoff` and `handoff_chars` are
not read), and its requests are unchanged. Its accept page shows only the
approach, so a person can accept a plan without seeing its handoff (up to
65,536 characters) until the task page (P8–P10) shows it; `handoff_chars` lets
it say "has a handoff (N characters)" without reading the text.

## Accepting or discarding a plan

A draft becomes the task's plan by `accept_plan(task, version, expected_rev,
expected_accepted)` (#2481 M4c). One commit writes the task, naming the version
(`accepted_plan`), the plan, `accepted` with who accepted it and when, and the
version accepted before, now `superseded`; all three share one stamp. Accepting
an older draft over a newer accepted version is a rollback: the newer one is
superseded. A superseded plan is never accepted again: propose it anew.

Both pins are required. `expected_rev` is the `rev` that `show(task,
version)` gave, so what is accepted is what was read: a draft changed since is
refused (`plan_changed`), also when an edit lands while the acceptance is being
written. `expected_accepted` is the task's `accepted_plan` as it was read
(`null` while it named none), so an acceptance never supersedes one its caller
did not see: a task that names another version now is refused
(`accepted_plan_changed`, with the version it names now as `accepted`), and a
rollback is a knowing choice. The answer gives the `task`, the `plan` and the
version it `superseded` (`null` when none was accepted).

`discard_plan(task, version, expected_rev)` discards a draft: it is kept, as
`discarded`, and only it is written. `expected_rev` is optional, as for
`edit_plan`. The answer gives the `plan`.

On a task someone holds, only the holder of a live claim accepts or discards
(`not_holder`; the holder of an expired claim gets `claim_expired` and renews
first, and anyone else takes the task over with `take_over` first: their
`not_holder` on an expired claim carries `expired: true` and says so). On a task
nobody holds, anyone does. Both are refused for a done or archived task
(`task_finished`), for a version that is not a draft (`plan_not_draft`) or that
the task does not have (`unknown_plan_version`), and while the task's file or a
file named as one of its plans is set aside. An acceptance is also refused while
a run of the task is going (`run_open`) or a file named as one of its runs is
set aside (it may be the open one), and while the task's plans or runs
contradict each other (`plans_inconsistent`, `runs_inconsistent`): mend them
first.

## Starting a task

`start(task, plan_version)` hands a task to a swarm (#2481 S4c). It is the agent
tool's alone: only an agent session can host a swarm, so `quecto board start`
is refused ("start needs an agent session: ask a quecto agent, or use Pocket").

One commit writes the task and its next run, task first: a ready task is
claimed by the starter (a task the starter holds has its claim renewed), moved
to `in_progress`, and run 1 (or the next) of its accepted plan begins, all at
one stamp. Then the run's coordinator is launched in a new container with the
plan's brief: the run to create, its task items in dependency order, each
title exactly as the board matches it back, and evidence revisions as full
commit ids, pushed before it reports a run that succeeded (see "Ending a run"). The
coordinator gets the plan's
`coordinator` role's model and effort; other roles' models are not applied yet,
and the answer lists them as `not_applied`.

- `plan_version` is the accepted plan the caller confirmed. A task that names
  another by the time the start is written is refused (`accepted_plan_changed`),
  so only a confirmed plan is ever launched. A task with no accepted plan is
  refused (`no_accepted_plan`), and so is one with a run going (`run_open`).
- A plan a swarm cannot take is refused before anything is written (`brief`,
  with the brief's own code: `team_too_large`, `team_unsupported`,
  `no_task_items`, `item_too_large`,
  `brief_too_large`, and `coordinator_model` for a coordinator role whose model
  does not name its provider as `provider/model`: none is guessed; the same
  rule refuses such a plan when it is proposed or edited, and is kept here as a
  backstop).
- A session that may not spawn starts nothing (`no_launcher`): `start`
  launches only what the session's own `spawn` would, and asks again as it
  launches.
- A swarm runs only in a container of its own repository
  (`container_refused`): before anything is written, `start` selects the
  container config the coordinator's `spawn` would (the checkout's own
  `standard`, else the labelled default) and confirms its `--repo` names
  the same repository as the checkout's `origin` (host, owner and name,
  case-insensitive; `https`, `ssh` and scp forms, `.git` and trailing
  slashes alike). A global default serves only the repository it names. A
  config that names no repository or more than one, or one either side of
  which cannot be read, is refused too: quecto cannot confirm it. The
  reason names both repositories and the fix, `quecto container init
  --project <dir> --repo <url>`. A session with no container-config
  selection is refused the same way. Any other fault in selecting the
  config (no default, an unknown or altered entry, a withheld overlay) is
  not judged here: the launch refuses it as `launch_failed`, as before.
- `spawn` with `coordinator: true` applies the same repository rule before
  its create script runs, and only a process launched as a coordinator
  (`--coordinator`, as that spawn launches one) may `swarm` `create`: any
  other agent, a plain container child included, is refused. So every
  swarm, a board start's or an ad-hoc one, runs in a container whose
  repository was confirmed.
- If the coordinator does not launch, the call is cancelled (after the write,
  then nothing is launched; or during the launch, then the launch is dropped
  and a coordinator that came up anyway is stopped), or the launch fails
  unexpectedly, the start is undone as far as it can be: the run is folded
  back as `failed`, which moves the task in the same write (#2705 T5): back
  to the review or blocked status the start took it from, its claim kept (so
  a review is still completed when its pull requests merge), or else back to
  ready, released. The undo writes even when the call was cancelled.
  `launch_failed` gives the launch's `reason`, the `run`, `run_state`
  (`folded` when the undo folded it back, `ended_already` when another writer
  had ended it, `open` when it could not be folded back: end it with
  `end_run`, below, or `refused` when a board server refused the undo, its
  code and words in the `reason`: `show` the task, then `end_run` the run as
  failed), `released` (`true` when the undo released the task to ready,
  `false` while it is still held, `null` when another writer's end moved it,
  or after a refused undo, unknown: the task is still held and nothing moved
  it) and `restored` (`null`, or the `review` or `blocked` status the
  undo moved it back to).
  A known gap: when the undo's own write fails (`run_state` `open`), the run
  is ended later by `end_run`, which knows nothing of the start, so a task
  the start took from review goes to ready, released (one from blocked can be
  kept blocked with `outcome stopped, stop blocked`). To finish such a
  review: `move(task, to claimed)`, then `done_without_run(task, reason)` once
  its pull requests have merged, or plan and start it again.
- A coordinator that came up outside a container is stopped, and the start
  undone. A launch waits for spawn at most 15 minutes.
- Nothing is undone while a coordinator may still be running: one that came
  up wrong and could not be stopped (its stop failed, or took past 30
  seconds; a slow stop still goes on to its end), or one the session still runs after a launch that was cancelled,
  panicked or timed out. Then `launch_failed` names it as `still_running`,
  `run_state` is `open` and the claim is kept: kill it (`agent_cmd kill`),
  then `end_run` the run as `failed`.

Once launched, the run is the swarm's, and the task stays held by its starter
until its holder ends the run as "Ending a run", below, says: close a run
paused holding an outcome, stop the coordinator, `end_run`, and renew the
2-hour claim, which nothing renews while the swarm runs. A `renew` writes only
from halfway through the claim's own lease on: before then the holder's renewal
writes nothing and answers the task as it is, its claim's expiry unchanged (its
`board_op` record notes `attempts: 0` and the claim it found). Every `renew`
answer gives `renew_after`, the time halfway through the claim it holds.

A harness the pocket relay launches for a board task writes the board as the
relay's bot: `QUECTO_BOARD_COMMITTER`, `name <email>` (`quecto pocket
<pocket@quecto.invalid>`), names who the board's commits and claims are by,
for every write of that harness's board, and nothing else git does: the
agent's own commits stay its clone's. A value not so shaped is refused
(`no_committer`), never ignored for the clone's own.

The answer gives the `task`, the `run`, the `coordinator` (`agent`,
`environment_ref`), `not_applied` (`role`, `model`, `effort`), `claim`:
`fresh` when the start took it, `held` when the starter held it already,
`renew_after`: the time halfway through the claim's lease, from which a `renew`
is due,
and, on the agent tool, `steps`: the steps for running the run to its end, as text for the agent that
started it, the task's id and the run's number filled in.

On the agent tool, every refused start's `error` carries `steps` too, what its
caller does next: for `launch_failed`, the undo by its fields, the task's id
filled in; for `contended`, to call again up to three times; for any other
refusal (the board's, an invalid request, a request the strict read refused
whose op reads as `start`, or a front end's own), to say why and stop. No other
op's answer carries `steps`, and these answers carry none: the CLI's (which
takes no `start`), a refusal of a request whose op cannot be read, and a call
that did not finish (cancelled, or ended without an answer). The words are the
Pocket start prompt's, moved there unchanged, so that prompt can be one line,
with one step added (#2666): before closing a run that succeeded, make sure
every commit its evidence names is on the repository's remote. Since #2705 T4,
which leaves the moves to review and blocked to the lifecycle, and #2705 T5,
whose run's end moves its task, the last step links a succeeded run's pull
request and says the run's end has moved the task, and the undo ends a run it
left open with `end_run` (outcome `failed`), which moves the task back to
ready, and says why the launch failed.

## Ending a run

Nothing records a started run's end by itself: the swarm never writes the
board, and the run holds its task until it ends, since a held task leaves its
holder only once its runs have ended. Its claim holder ends it, in this order:

1. For a run that succeeded, make sure every commit its evidence names is on
   the repository's remote (have the coordinator push any that is not): once
   the coordinator is killed, its container and any unpushed commit are gone,
   and `end_run` refuses the success (`not_pushed`, below).
2. When the run is paused holding an outcome, close it: `agent_cmd`
   `swarm_control` `close` on the coordinator. A cancelled run needs no close.
3. Stop the coordinator with `agent_cmd kill`. Until it has stopped, `end_run`
   answers `run_live`, naming the coordinator's `agent` to kill.
4. `end_run(task, run, outcome, expected_rev)` with `succeeded`, `stopped`
   or `failed`. The end moves the task (see "Where a run's end leaves its
   task", below).
5. Renew the claim before it lapses: it lasts 2 hours, and nothing renews it
   while the swarm runs. If it has expired, renew it if it is yours
   (`renew`), or take the task over if it is another's (`take_over`), then
   `end_run`.

The same ends a run no swarm will ever finish: a start whose coordinator never
launched and whose undo could not end the run (its answer's `run_state` is
`open`), or a start that wrote and then stopped (the process died) before it
launched.

`outcome` is one of `succeeded`, `stopped`, `failed` or `lost`; any other word
is refused. The holder of a live claim on the task ends the run as `succeeded`,
`stopped` or `failed` (`not_holder` for anyone else, `claim_expired` for a
holder whose claim lapsed: renew first). The run is recorded from its swarm's
board as it stood when the coordinator left its container: each of its plan's
task items in the state its swarm item reached (matched by its brief title, as
a fold-back matches it), with its owner and evidence, and the swarm's observed
tokens as its cost. The swarm's board lives in the coordinator's container and
goes with it once a closed run's coordinator is killed, so the final-member
teardown reads it first, when the run has ended (succeeded, failed,
budget-exhausted, blocked or cancelled), and keeps what a fold-back reads of it
in the harness's memory, under the session
and the coordinator's spawn label (`board-<task>-run-<n>`; no other label's
board is kept, so an ad-hoc swarm's never is). The agent tool's `end_run` finds
it by that label, with no memory of the launch. A board is kept only when its
projection holds every item (one that left items out is logged and not kept;
members or evidence left out do not matter), only the newest 16 are kept, and
one is dropped once its run is recorded. A run still running or paused is kept
only when its owner ended the coordinator on purpose (a one-shot parent's end,
an `agent_cmd kill`; #2674), as far as it got, apart from an ended one: a
`stopped`, `failed` or `lost` end records its items from that board (finished
items with their owners and evidence; their evidence commits are recorded
unchecked, as only a success is judged by where its commits are), and a
`succeeded` end never does (it answers `items_not_done` with `report`
`"none_kept"`). Only the newest 16 boards are kept in all, and past that a left
board goes before any ended run's. On any other end (the
coordinator exiting by itself, a harness shutdown) a running or paused run's
board is not kept: it may yet change. Where no board was kept, the run is recorded
as a swarm that ended so and reported nothing would be: each task item as todo,
its cost unknown. That is so from the CLI, for a run another session launched,
for a board that could not be read whole, and for the gaps a restart leaves:
the kept boards live in memory, so a harness restarted between the
coordinator's teardown and `end_run` has lost its board, and one that no longer
holds the coordinator as its subagent (with its label) when the box is torn
down keeps none.

A board is kept once the coordinator's teardown has ended its workers, and
each worker's confirmed death blocks the items it held ("worker death
confirmed; coordinator recovery required", or its abrupt-exit form). Such a
blocker is read as the teardown's, not the item's (#2685): an item blocked by
it is folded back as it most likely stood before, in review when it holds
evidence (a submission writes it, and a submitted item is never blocked by its
worker), else doing. An item blocked for any other reason is folded back
blocked. When an item was created again, the copies are ranked by the state
the board holds, the newest by where it stood before a death that blocked it,
so a copy a death blocked never outranks the copy that replaced it, and the
newest copy is never outranked by an older one nobody claimed. The reading is not proof: a submission released and claimed
again keeps its evidence, so it is folded back as in review; a worker that
died mid-run and was never recovered is read the same way; and a blocker of
the item's own that the death overwrote is folded back as doing.

A run ends as `succeeded` only when every task item of its plan is done there:
otherwise the end is refused (`items_not_done`, a rule refusal naming the
`run`, the `items` not done and the `report`) and nothing is written, so the
board never shows a succeeded run over work it shows undone. With `report`
`"kept"`, the kept board showed those items undone: the run is ended as
`stopped` instead. With `"none_kept"`, no board was kept to show them: the
refusal asks the user whether the work was done before the run is ended. A run past the board's bytes with its evidence is ended
once more without it, as a fold-back is.

A run ends as `succeeded` only over work that left the swarm's container
(#2666): every commit the evidence of its recorded task items names must be on
the board's remote, reached by one of its branches or tags. A revision's commit
is its one word (split at whitespace, `@` and `/`, surrounding punctuation
trimmed) that is wholly hex digits, 7 or more (`<sha>`, `` `<sha>`. ``, `commit
<sha>`, `<sha> (main)`, `owner/repo@<sha>`), or, when several are, the one of
them with 12 or more (`<sha> on 20261021`), read in lower case, so the same
commit in two cases is asked about once; a revision with none, or with two of 12
or more, names none. Only a full commit id of the
repository's format (40 hex digits; 64 in a SHA-256 repository) or a prefix of
at least 12 is checked: a 64-digit digest in a SHA-1 repository, a 7 to 11 digit
short id, and evidence that names no commit (an artifact's digest, a red-test
bundle, a branch or tag name, `working tree`) are accepted unchecked, so a done
item whose evidence names no checkable commit does not refuse the success. The
answer of a success recorded from a kept board gives `unchecked`: how many of
its recorded items' evidence entries name no commit the push rule checked (each
entry counted, repeats included), so a pass with nothing checked shows. Otherwise the
end is refused (`not_pushed`, a rule refusal naming the `run` and how many
commits, `revisions`, are not there) and nothing is written, so the board never
shows a succeeded run over a commit made in the container and torn down with
it. Every other rule judges the end first (the claim, the run, its items), so
the remote is asked only for an end that passes them, and it is asked once,
before the write begins. To judge it, the remote's branches and tags are listed
(`ls-remote`); each commit is walked against the tips the clone holds, and only
when they do not reach it are the tips it lacks fetched by id, moving no ref and
writing no `FETCH_HEAD`. A remote that cannot be read refuses the end as
unavailable; a shallow clone reads a commit past its boundary as not there.

The check comes after the container is gone (the kept board is read as the
coordinator leaves), and the kept board is the swarm's record, not its commits:
a commit that was never pushed went with the container. So the commits are
pushed before the run is closed: the coordinator's brief asks it to give each
evidence revision as a full commit id and, before it reports a run that
succeeded, to push those commits as they are to a branch on the repository's
remote, naming that branch and its head (evidence it deliberately does not
push, such as a red-test bundle, names the bundle, not a commit); and the
`start` answer's steps ask the parent, before it closes a run that succeeded,
to make sure every such commit is on a branch or tag of the board's remote. And
the owner's kill (`agent_cmd kill`) of a board run's coordinator whose run
ended succeeded keeps its environment (retained, as an unfinished run's is;
`kill_container` removes it) while the commit the run completed at (`completed
at <revision>`) is not on the board's remote, or that cannot be told: the board,
the session's board repository or its remote cannot be read (each git call
within 15 seconds), or the completion names no one commit the push rule checks
(the same reading as the evidence's). The reason names the environment and its
git checkout (the create's advertised `checkout`, `<workspace>/repo` for the
shipped scripts, else the workspace): "board run succeeded at <revision>, which
is on no branch or tag of the board's remote: environment <ref> kept so its work
can be pushed from its checkout <path>; kill_container <ref> to remove it once
pushed". The work survives, and can be fetched from that checkout and pushed.
The kill's own answer says so (#2681): beside `result` and `killed` it carries
`"kept": [{"environment": "<ref>", "reason": "<the reason above>"}]` for each
environment its teardown kept (each reason redacted and clipped to 400
characters, at most 20 listed, `omitted_kept` counting the rest), and `quecto container ls` shows the same reason
(a `note: <ref> retained: <reason>` line on stderr, the `retained` field of
`--json`). The board's remote
is the `origin` of the repository the session's board tool works on. Only that
kill keeps it: a run closed and then left to the session's end (its teardown of
everything, or a harness shutdown) is torn down as before, so push first. If a
refused end's commits can still be pushed, push them and call `end_run` again;
if they went with the container, end the run as `failed`. Commits are judged against the board's own
remote only: a run whose container cloned another repository (a container
config naming another URL) is refused until its commits are on the board's
remote too, and is otherwise ended as `stopped` or `failed`. A stop, a failure
or a loss is never judged by it.

As `lost`, the run is ended by the holder of a live claim, or by anyone when the
task has no claim; whoever finds the claim expired takes the task over with
`take_over` first, and then ends the run as its holder. A lost run records what
is known of it, as a stop does (#2657): its task items, owners, evidence and
cost from its swarm's kept board, or, with none kept, each task item as todo
and its cost unknown. (A run ended as lost by the completion that makes its
task done records none.) `expected_rev` is optional: when given, the task
must still be as it was read (`stale`).

### A session that ends with its run open

A one-shot `quecto agent -m` session that started a task, and whose reply ends
while the run is still open, does not leave it `running` with no swarm behind
it (#2655). Once the run ends in an orderly way (the model finished, or
`--max-time` stopped it) and its children have settled, every run the
session's own starts launched that the board still shows open is ended as
`stopped`: its swarm stopped with the session that ran it, which can account
for it, so it is not `lost`. Each end goes through the same path as an
`end_run` from the agent tool, with its rules (the holder's live claim) and its
liveness check (a coordinator still running keeps the run open), and its
`board_op` record names the client `session_end`. The run's items are recorded
from the board its swarm had reached as the session ended its coordinator, kept
as above, so items its workers finished keep their state and evidence. The
session says on stderr what became of each: "board: run <n> of task <task> was
still open as this session ended: ended it as stopped", or, when the end was
refused, "…, and ending it as stopped was refused (<code>): end it with
end_run", `<code>` being the rule's own (`not_holder`) when the board's rules
refused it. When the board cannot be read it says "board: the runs this session
started could not be read as it ended (<why>): end any still open with
end_run". A run another
session launched, or one the board shows ended, is left alone; the task stays
as it is, held until its claim lapses or its holder moves it. A run that
failed (a provider error), a crash, a signal, and a harness the TUI or the
socket protocol runs end none: such a run is still ended by hand.

Never end a run whose swarm is still running. An `end_run` from the agent tool
is refused (`run_live`, with its `task`, `run` and the running coordinator's
`agent`) while the asking session holds any running subagent with the run's
label (`board-<task>-run-<n>`). A coordinator another session launched is not
seen, and the CLI sees none: its answer says `liveness: "not_checked"` (the
agent tool's, `"checked"`), with a `liveness_note`: "this end was not
checked against running swarms: if the run's coordinator still runs, stop it".

Only the run is written; the task stays as it is, for its holder to move on
(back to `ready`, say). A run that has ended is refused (`run_ended`), as is
one the task does not have (`no_such_run`). The answer gives the `run`, as
`show` gives a run in full, `liveness`, and, for a run ended with no swarm board
kept (as lost too), `items_recorded`: "items recorded as todo: no swarm report".
A run recorded from its swarm's board carries no `items_recorded`: its items
show where each stood. `quecto board end-run` takes the same fields. The
answer's `status` is where the end left the task.

### Where a run's end leaves its task

A run's end writes its task's status in the same commit as the run (#2705 T5),
whether `end_run` ends it or a fold-back (a start's undo included):

| The run ended | The task |
|---|---|
| `succeeded` | review, its holder keeping the claim |
| `stopped`, its swarm's kept board ended blocked | blocked, its holder keeping the claim |
| `stopped` with no board kept, and `stop: "blocked"` given | blocked, its holder keeping the claim |
| `stopped` otherwise (cancelled, or no board kept and no `stop: "blocked"`), `failed` or `lost` | ready, its claim released |

`end_run` takes an optional `stop`, `blocked` or `cancelled` (the default): how
a `stopped` run stopped when no board its swarm ended on was kept to say it
(the CLI's `--stop`). A kept board its swarm ended on wins; a board kept while
the run still ran or was paused (read as cancelled for its items alone) does
not, and the caller's `stop` decides. Any other outcome refuses it
(`invalid_request`, field `stop`).

Only a held task moves (claimed, in progress, in review or blocked); a task in
any other status, or already where the end moves it, is left, and the run is
written alone. Nothing follows the end: while a run is going, `move` refuses
in_progress to ready, review or blocked (`use_lifecycle`, `end_run`). An
`end_run` of a run that has ended answers `run_ended` before any claim is
checked, as the end may have released the claim. A start's undo is a
fold-back too, but it puts a task the start took from review or blocked back
there (Starting a task, above).

The Pocket relay's Stop (quecto-pocket `stop_board_run`) ends the run as
`stopped` through `quecto board end-run`, which keeps no board, so the task
goes back to ready, released. It then reads the task again and moves it back
to ready only while it is still in progress or claimed: it finds it ready and
moves nothing. A legacy task left in progress with no run going (no run at
all, or only runs that have ended, as an older quecto left them) shows
`in_progress_without_run` in `show`, and its holder releases it with
`move(task, to ready)` (the claim live or lapsed); the Stop's own move does
the same for one the relay's bot holds.

## Status moves

`move(task, to)` makes only the moves people make (#2705 T4); the task's
lifecycle makes the rest. Every move `move` makes:

| Move | Who may make it |
|---|---|
| draft to ready, ready to draft | Anyone |
| ready to claimed | Anyone, taking a two-hour claim |
| claimed to ready | Its holder, the claim live or lapsed (a release) |
| in_progress to ready | Its holder, the claim live or lapsed, only while no run of it is going: no run at all, or only ended ones (a legacy state, #2705 T5). With a run going it answers `use_lifecycle` (`end_run`) |
| review to done | Its holder, the task linking no pull request or change (Mark done) |
| review to ready | Its holder while the claim is live, anyone once it is not, with a `reason` |
| blocked to ready | Its holder while the claim is live, anyone once it is not |
| to archived | As Archiving a task says |

Any other move is refused, whoever asks, before anything is read. A move the
lifecycle makes answers the rule `use_lifecycle`, with `action`, `from` and
`to`, naming the action to use:

| Move | `action` |
|---|---|
| ready, claimed, review or blocked to in_progress | `start`: `start(task, plan_version)` is the only way to in progress |
| in_progress to review or blocked; in_progress to ready while a run is going | `end_run`: the task's run decides it (#2705 T5) |
| claimed to blocked | `release`: blocking is a run's outcome, so work without a run is released with `move(task, to ready)` |
| ready or claimed to done | `done_without_run` |

Every other pair answers `not_allowed`. A merged pull request completes a task
only from review (Claiming and completing a task, below). A start undone
after its coordinator did not launch takes its task back (to review or blocked, or ready) by its
run's end, never by `move`.

### A reason to move back

A move back to ready from review takes a `reason`: `move(task, to ready,
reason)`, one line of at most 1,000 characters, normalised as every entered
line is. The agent tool refuses the move without one (`reason_required`, with
`from` and `to`). `quecto board` lets it through without one: the Pocket relay
moves a task back through it with no reason until its task page sends one, as
its entry policy lets the relay's free text in. A reason given to any other
move is refused (`invalid`, field `reason`): "is taken only by a move from
review back to ready".

A task keeps its reason (its file's `Reason` field) only beside the move that
took it: a ready task's, moved back from review, or a task's done without a
run (kept when it is archived). Every other status move clears it. Answers do
not show it yet, as they hold back `resolution`.

### Done without a run

`done_without_run(task, reason)` finishes work done with no run of the task's
plan (an answer, a fix made by hand): a ready task by anyone, a claimed one by
its holder while the claim is live. The reason is required (`invalid_request`,
field `reason`, when it is missing; `invalid`, field `reason`, when nothing is
left once it is normalised as one line, "must say why", or when it is more
than one line or past 1,000 characters, "must be 1 to 1000 visible characters
on one line"). The task is resolved `done_without_run`, keeps the reason, and
leaves its claim.

It is refused from any other status (`not_allowed`), for a task others took
over (`superseded`), while a run of it is going (`run_open`), and as a claim
and an archive are refused: while a task it depends on is not done or archived
(`dependency_not_done`), and while a child task is claimed, in progress, in
review or blocked (`children_active`). A task that links pull requests is done
so only once every one has merged (the owner's choice): their states are read
from the host as `complete` reads them, within the same 30-second budget,
and only once every other rule allows the task. One not merged (open, or
closed unmerged) refuses it (`prs_open`, naming them in `prs`); a host that
cannot be read in time answers `pull_requests_unavailable`, one that does not
know a pull request `pr_state_unknown`, and none at all `no_pull_request_host`.
A change linked and not merged refuses it the same way (`changes_unmerged`).
A merged pull request still completes a task by itself only from review.
`quecto board done-without-run` takes the same fields. A board server will
need to read the pull requests' states before its write lock, as it will for
`complete` (#2705 B7/B8).

### The task's file

A task holding a reason is written at `schema=3`, its description in either
layout (its sections, or legacy text as its `## Description`), with a
`| Reason | ... |` field after `Resolution`; a task without one keeps the
schema its description's form picks. **Deploy order:** as for schema 2,
upgrade every reader, the Pocket relay's host included, before any task is
moved back with a reason or done without a run. An earlier build sets a
schema-3 task aside as written by a newer quecto: never `clear_set_aside` it
from an old build.

## Claiming and completing a task

A ready task is claimed (`move(task, to claimed)`, or by a start) only once
every task it depends on is done or archived, as `list_ready` lists it: otherwise
the claim is refused (`dependency_not_done`), naming the tasks that hold it back:
those not done yet in `tasks`, and those not on the board (or set aside) in
`missing`.

Only the claim's holder renews it, releases it (`move(task, to ready)` from
claimed, or in progress with no run going; a task whose run is going is
released by the run's end) and links or unlinks a pull request: anyone else is
refused (`not_holder`), and once the claim has expired that refusal carries
`expired: true` and says to take the task over first with `take_over(task)`,
on every holder-only action alike (#2683). A link is judged on the task first:
someone who may not link is told so, whatever else is set aside. Only then is a
link by the holder of a live claim refused while a task file is set aside
(`set_aside`, or `set_aside_file` for a kept file whose name spells no task),
since that file may link the same pull request and a pull request is linked to
one task once (#2686).

`complete(task)` completes a task in review whose linked pull requests have
all merged, reading their states from the repository's host (GitHub, or the
configured `GH_HOST`); a task in any other status is refused before anything
is read (#2705 T4): a ready or claimed one with `use_lifecycle`, naming
`done_without_run`, any other with `not_allowed`. A task that links no pull request has
nothing to read there: it is
refused by the rule itself (`no_prs`, whose message names the way on), with or
without a host, and its holder finishes it by moving it from review to done. Only a task that links pull
requests needs the host, and with none it is refused (`no_pull_request_host`).

## Archiving a task

`move(task, to archived)` archives a task (Pocket's Delete task): the board
never deletes one, and never moves one out of `archived`. Anyone archives a
`draft`, `ready` or `done` task; a `claimed`, `in_progress` or `blocked` one is
refused (`not_allowed`). A task in review is archived by its holder while the
claim is live, and by anyone once nobody holds it (its claim lapsed or gone),
as it goes back to ready (`not_holder` for anyone else while the claim is
live); its claim is cleared, as it is when the task is done. A draft, ready or
review task is archived on review to done's terms: it is refused while it links
a pull request (`prs_linked`) or a change does (`changes_linked`), so releasing
a task first (review to ready, on to draft) never archives its links either,
and an archived task pins none that has not merged (unlink them first). Any
task is refused while a run of it is going (`run_open`), or while a file named
as one of its runs (or, for those three, its changes) is set aside (it may be
the open one): end the run first (Ending a run, above), so an archive never
races a live run. A task is refused while a child task (one naming it as its
`parent`) is claimed, in progress, in review or blocked (`children_active`,
naming them in `tasks`): finish or release them first. It is refused too while
a task file is set aside (`set_aside`), since that may be a child of it. An archived task
keeps its done one's resolution (`completed`), is `superseded` when others took
it over, and otherwise has none.

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
as given. It is for the pocket relay to set, for `quecto board` and for the
harness it launches, whose agent tool's calls are then recorded as the relay's;
an older quecto ignores it.

| Field | Meaning |
|---|---|
| `session` | As on every event-log record: the agent's session for the tool; `board-cli:<UTC date>` for `quecto board` |
| `turn` | Always `null`: a board call is filed under no agent turn |
| `op` | The op the request named: `bootstrap`, `list`, `list_ready`, `list_takeable`, `show`, `create`, `update_draft`, `move`, `renew`, `take_over`, `link_pr`, `unlink_pr`, `complete`, `clear_set_aside`, `propose_plan`, `edit_plan`, `accept_plan`, `discard_plan`, `end_run`, `start`, `done_without_run`, `set_outcome`; `unknown` for a request that names none, or none the board has. A request the agent tool does not take (`bootstrap`) records its op, refused, and a `quecto board` line refused as it is read records the op it named before the refusal |
| `front_end` | `tool` (the agent `board` tool) or `cli` (`quecto board`) |
| `client` | For the tool `agent`, for the CLI `cli`, or, for either, as `QUECTO_BOARD_CLIENT` names it: `pocket_relay` or `other`; `session_end` for an `end_run` the harness makes as a session ends with a run it started still open |
| `board` | The board the request reached, once its operations were built: an opaque id, 16 hex digits of the SHA-256 of its remote's repository as its fetch URL names it: the host in lower case (with its scheme's own default port left out: ssh 22, http 80, https 443, git 9418, an scp-style `user@host:path` counting as ssh; another port, or another scheme's default, is kept) and the path with its case kept, the scheme and any userinfo left out, an scp-style `user@host:path` read as host and path, a trailing `/` and `.git` dropped; a local remote, a path or a `file://` URL, is the canonical absolute path it names, a relative one resolved from the clone's top level (a bare clone's git directory). So the https, ssh and `.git` spellings of one repository give one id, and one daily `board-cli_<date>.jsonl` holding several repositories' records tells them apart. The id names no remote, but it is opaque, not secret: anyone who knows a repository can compute its id |
| `access` | `read` or `write`: what the op does, even when it was refused before it was read |
| `outcome` | `ok` or `refused` |
| `code`, `rule` | For a refusal, the answer's error code (`invalid_request`, `rule`, `not_found`, `stale`, `contended`, `clock_ahead`, ...) and, for `rule`, the rule's code (`not_holder`, `claim_expired`, `not_allowed`, ...). `internal` is no answer's code: an agent tool call whose work ended without an answer (it panicked) is recorded as it unwinds, refused as `internal`. A `quecto board` panic aborts the process and writes no record |
| `field`, `fault`, `unexpected_count` | For `invalid_request`, `invalid_task` and the rules `invalid` and `invalid_plan`, which field was wrong: a schema field as the board's table spells it (`depends_on/2` is `depends_on`, `items/3/criteria/0` is `items`), `op`, `request` (the request as a whole; a plan's text past the most a plan holds, `invalid_plan`'s `text`, is recorded as `request`, `too_large`), or `other`; and how: `missing`, `unexpected`, `wrong_type`, `invalid_value`, `too_large`, `not_json`, `duplicate_key`. A key no schema of the op names is `other`, `unexpected`, and `unexpected_count` counts such keys, never naming one, even when another op takes a field of that name |
| `task`, `pr` | The task the op's `task` names, and the pull request `link_pr` or `unlink_pr` names, once the request was read |
| `head` | For a write, the commit it made; otherwise the board's head an answer named. Only ever a full commit id git produced. A `link_pr` that completes its task records its own write's commit; the completion's commit is not recorded |
| `listed`, `shown`, `truncated_more`, `set_aside`, `set_aside_more` | For an answered listing: the tasks it listed, those its answer showed and those it left out, and the set-aside files it listed and left out. The agent tool's listing is cut to 32 KiB; the CLI leaves nothing out |
| `plans`, `runs` | For an answered `show`: how many versions of its plan and how many runs the task holds, as the board read them; counts only, never their text |
| `plan_version`, `items`, `roles` | For `propose_plan`, the version it wrote, only once written. For `edit_plan`, `accept_plan`, `discard_plan` and `start`, the version it named, also when the board refused the change. For `show(task, version)`, the version it named once the task was read, also when that version was refused; a `show` of a task the board refused records none. A request refused as it is read records none. For `propose_plan` and `edit_plan`, once written, how many task items and team roles the plan holds (`roles` is 0 with no team); counts only |
| `plan_state_from`, `plan_state_to` | For `accept_plan` and `discard_plan`: the state of the version it named as the change last read it (`draft`, `accepted`, `superseded` or `discarded`; none when the task has no such version, or the read was refused first), and the state it wrote (`accepted` or `discarded`; none for a refused change) |
| `run`, `run_outcome` | For `end_run`, once the request was read: the run it named and the outcome it ends it with (`succeeded`, `stopped`, `failed` or `lost`), also when the board refused the change |
| `status_from`, `status_to` | For a write of a task (`create`, `update_draft`, `move`, `done_without_run`, `renew`, `take_over`, `link_pr`, `unlink_pr`, `complete`, `set_outcome`): the task's status as the write last read it (none for `create`), and the status it moved to: a `move`'s target (`done_without_run`'s `done`), made or refused, otherwise the status of the task it wrote (`create`'s `draft` or `ready`, `complete`'s `done`), so a refused write other than `move` writes no `status_to`. A `link_pr` that completes its task records its own write's status here; what came of completing it is in `completion`. An `end_run` records only `status_from`, the status of the task whose run it ends: it writes no task |
| `claim` | For a `move`, `renew`, `take_over`, `link_pr`, `unlink_pr` or `end_run` of a task that holds a claim, the claim as the change found it, never its holder: `held_s` (seconds since it began, never below zero: 0 when the writer's clock reads before that), `left_s` (seconds until it expires, below zero once it has lapsed) and `holder_is_writer` |
| `attempts`, `races_lost`, `pause_ms` | For a write (every op that writes but `bootstrap`): the attempts it began (at most 3), how many of them lost the race to another writer (`races_lost`, never a run's `lost` outcome), and, when it paused between attempts, the time it paused in milliseconds. A `link_pr` that completes its task counts only its own write's attempts here; the completion's write is counted in `completion_attempts` |
| `lock_races` | For a write (a `clear_set_aside` too) whose fetches found the board's tracking ref locked by another writer in the same clone, how many times they fetched again (The board branch, above); absent when none did. The store counts them, so calls that run at once in one process (the agent tool's) may each count the other's |
| `swarm_report` | For an `end_run` that wrote, whatever its outcome: `kept` when its front end kept the board the run's swarm left, `none_kept` when it kept none (a held end records every item as todo), `not_checked` when it keeps none to ask (the CLI). An end that folds a kept board's items onto its run also writes its own `board_fold_back` record, with the run's items by state |
| `completion`, `completion_code`, `completion_rule`, `prs_linked`, `prs_pending` | After a `link_pr` to a task in review that links pull requests: `done` (every one merged), `pending` (some not merged yet, counted in `prs_pending`) or `failed` (the host, the store or a rule failed it, with its error code in `completion_code` and, for a rule, the rule's code in `completion_rule`); `prs_linked` counts the pull requests the task links. `complete` records only `done` or `pending`: a `complete` refused for any other reason records no completion, and its `code` says why |
| `completion_attempts`, `completion_races_lost`, `completion_pause_ms` | For a completion run after a `link_pr`, its own write's attempts, lost races and pause, as `attempts`, `races_lost` and `pause_ms` count the change's |
| `normalised` | For `create`, `update_draft`, `propose_plan` and `edit_plan`, what the board's normaliser changed in each text field the request gave (`title`, `description`; a plan's `approach`, and its `items`: every kind of change made to any task item's title or criteria; a `create` that gave no description, or an empty one, tells none, nor does an `edit_plan` that gave neither an approach nor task items): a list of `crlf`, `lone_cr`, `line_break`, `chat_space`, `tab`, `edge_blank_lines`, `trailing_space`, `edge_space`, empty when it kept the field as written; never the text |
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
(written to `tracing` at `INFO` as `board fold back`): a start's undo, and an
`end_run` that folds onto its run the items of the swarm board its front end kept
as the coordinator left (#2667): any end with a board kept, a `lost` one too. An `end_run` with
no board kept folds nothing reported, so it writes none; its `board_op` says
`swarm_report: none_kept` (or `not_checked`). A record holds no titles,
owners or evidence, only kinds and counts.

The run's own owners and evidence are scrubbed before they reach the board,
which is public, so a line in doubt is dropped whole (counted in
`evidence_dropped`; an owner in doubt is left out), never rewritten further. A
line is in doubt when it holds:

- a part past its board limit in characters (500 for an evidence part, 100 for
  an owner): it is never cut, since a cut may strip the `@` that marks a
  secret;
- an invisible character: a format character or another default ignorable one
  (a soft hyphen, a direction mark, a zero-width space, a variation selector, a
  tag);
- a credential word as any part of a word, once the text is folded (NFKC, so a
  fullwidth form reads as its letters, then lower case): `authorization`,
  `bearer`, `password`, `passwd`, `pwd=`, `secret`, `token`, `api_key`,
  `apikey`, `api-key`, `credential`, `private_key`, `private-key`, or `basic`
  before anything but a letter (so "basically" is kept);
- an address whose fragment is no short anchor (up to 64 letters, digits, `_`,
  `.` or `-`), whose path holds `%25`, or a bare host whose word holds a `;`;
- an at sign (`@`, fullwidth `＠`, small `﹫`, or `%40` encoded once or more)
  outside an address, judged after secrets are redacted, but in a `worker@1` or
  `HEAD@{1}` word.

Otherwise its spaces are made plain, and every address (`scheme://…`, `//host…`
or a bare `host.tld` before a `/`, `?` or `#`, in a code span or a link too)
that holds no at sign and has an unambiguous authority (an ASCII host and
perhaps a port of digits) keeps that authority, its path up to its first `?`,
`#`, `;` or `%3F`, and a short anchor such as `#L10-L20` straight after it; any
other address is written `scheme://***`. Secret-shaped spans are then redacted.

Accepted costs: a line that merely mentions a credential word ("basic CI-check
passed", "token budget") is dropped, so a task item about auth, token or
credential code may show little or no evidence (drops are counted in
`evidence_dropped`, not shown); a line holding an invisible character, or past
its limit, is dropped too; an address
with an at sign anywhere (an npm `/@scope`, vite's `/@vite/client`,
`?to=a@b.co`), an IPv6 or a non-ASCII host is written `scheme://***`; a line
holding an email, a `pkg@1.2.3`, an `@types/node`, a `git@host:repo` or any
other at sign outside an address is dropped. Known limits: a secret with no at
sign and no credential word (`-u admin:hunter2`), or in an address's path (a
Slack or Discord webhook), is kept, as is a credential word spelt with a
lookalike from another script (a Cyrillic `е` in `sеcret`) or `basic` glued to
letters (`basicAuth=…`); a bare `host.com;sig=…` with no `/`, a
bare host with no dot (`localhost/x?sig=…`), or one glued to a CJK letter or a
`_`, keeps what follows it. A line split by a line break is dropped whole.

| Field | Meaning |
|---|---|
| `task`, `run`, `board` | The task (its slug, redacted and capped as above), the run's number, and the board, as `board_op` names it |
| `outcome`, `code`, `rule` | `ok`, or `refused` with the error and rule codes a `board_op` would carry. A run that had already ended (folded back, or ended as lost by the merge that completed its task) is `ok` with only `attempts` and `races_lost` |
| `run_outcome` | How the run ended: `succeeded`; `failed` (the swarm failed or exhausted its budget); `stopped` (blocked or cancelled) |
| `items` | The run's task items by state: `todo`, `doing`, `review`, `done`, `blocked` |
| `unmatched`, `extra` | Plan items no swarm item matched by its title (`<slug>: <title>`) or, failing that, by its `<slug>: ` prefix, left `todo`; swarm items no plan item took (of several under one key, all but the most progressed, then newest), not kept |
| `evidence`, `evidence_dropped` | Evidence lines kept, and those left out: refused by the board's schema or file format, repeated, or past an item's 16 or the run's 256 KiB (owners counted first). A run that would leave the board, or its `runs/`, past the bytes a read takes is written again without any, every line counted dropped; any other invalid write is refused, with nothing left out |
| `cost_tokens` | The tokens the swarm was observed to spend |
| `attempts`, `races_lost`, `lock_races`, `bytes` | The writes' attempts, lost races and lock races (both passes, when the evidence was left out), on every outcome; and the bytes of the run's text |
| `head` | The commit the fold-back wrote |
| `duration_us` | The whole fold-back |

### Start records

Each `start` that reaches the board writes one `board_start` record besides its
`board_op`, under the same switch, gate and rules (written to `tracing` at
`INFO` as `board start`). Like every record, it holds kinds and counts only.

| Field | Meaning |
|---|---|
| `task` | The task started, as `board_op` names it |
| `client`, `board` | As on its `board_op` |
| `outcome`, `code`, `rule` | As on its `board_op`; a brief's refusal names its code as `rule` |
| `run` | The run begun, once written |
| `plan_version` | The accepted version the start named |
| `members` | The member limit the brief gives, its coordinator included; none when the plan names no team |
| `roles` | The plan's team roles |
| `items` | The task items the brief seeds |
| `not_applied` | The worker roles whose models were not applied |
| `claim` | `fresh` when the start took the claim, `held` when the starter held it |
| `launch_us` | How long the coordinator's launch took |
| `run_state`, `released` | For a start undone (its coordinator did not launch, its call was cancelled, or its launch failed unexpectedly): where its undo left the run (`folded`, `ended_already`, `open`, or `refused` when a board server refused the undo), and, when the start took the claim, whether it released it (`null` when another end moved the task, or after a refused undo: unknown, the task still held and nothing moved it) |
| `duration_us` | The whole call |

A start undone writes its fold-back's own `board_fold_back` record too, as
below.


## Settled done-when outcomes

Completing a task settles every implicit done-when outcome as met in the domain:
merged pull requests, merged changes, review-to-done and `done_without_run` all
share this rule. Completion refusals still leave both status and outcomes alone.
Old **sectioned Done** tasks with unchecked outcomes read as met, without a
migration commit or any write on read. Legacy prose stays byte-preserved, even
when it resembles a checklist. Unfinished tasks and unrelated archived tasks
are not normalized.

A finished task can record an explicit unmet outcome without reopening it:

```json
{"op":"set_outcome","task":"a","outcome_index":0,"met":false,"reason":"Owner deferred deployment","expected_rev":"r3"}
{"op":"set_outcome","task":"a","outcome_index":0,"met":true,"expected_rev":"r4"}
```

Copy `show.rev` for `expected_rev`: it pins the **exact board HEAD**, not just
the task's file or timestamp. Any intervening board write, including an
unrelated task's, makes it stale. Refresh and decide again; retries never
blindly rebase an index onto a changed board. `expected_updated` is not accepted.
The index is zero-based in the existing outcome order (duplicate text is fine).
Only a sectioned task in **Done** is eligible; no outcome is added or reordered,
no general finished-task edit is enabled, and Archived is not eligible.

An unmet outcome requires a normalized nonblank reason, at most 1,000
characters, still subject to the task's description and total text caps.
It must also fit the existing **canonical inline-Markdown text-cell grammar**;
normalization is not arbitrary-plaintext escaping. Use visible plain words,
emphasis, balanced code spans, or labelled links of the form
`[label](https://example.com/path)` (no title or userinfo). A code span must close
with a backtick run of exactly the opening width. For example, the reason
``Owner `deferred`` is rejected: close it as ``Owner `deferred` `` or remove the
backtick. To include a literal backtick inside code, use a wider matching fence,
for example ```See ``a`b`` ```.

Raw HTML/autolinks, bare URLs or emails outside code, images, full/collapsed reference links,
character references and dollar math are not supported. Put literal addresses
in balanced code spans or use a labelled https link. Pipes and literal
backslashes use the existing text-cell escaping on persistence; callers do not
pre-escape table pipes. Unrepresentable reasons are refused without writes;
the diagnostic identifies `reason` and gives canonical-text remediation (for
an open code span, close the matching backtick run or remove the backticks).
Restoring met removes the exception and refuses **any** supplied reason,
including null or blank. Missing, malformed or stale revisions, invalid indices,
wrong field types and ineligible tasks refuse atomically without writes.
Status, resolution, claim, task-level lifecycle `reason`, outcome text/order and
other description sections remain unchanged.

The CLI uses the same operation envelope and answers:

```sh
quecto board set-outcome --task a --outcome-index 0 --met false --reason 'Owner deferred deployment' --expected-rev "$REV"
quecto board set-outcome --task a --outcome-index 0 --met true --expected-rev "$FRESH_REV"
```

Answers retain `done_when[].outcome` and `done`. Only an explicitly unmet entry
adds `exception: {reason, by: {name, email}, at}`. `by` comes from the application's
configured writer, **a self-asserted git committer, not an authenticated human**;
`at` is the task's chosen `updated` write stamp. Neither can be supplied by the
request. Show, list and change answers share this representation.

Explicit exceptions persist at task **schema 4**, with this optional table after
Done when and before Links, both in canonical descriptions and task files:

```markdown
## Outcome exceptions
| Outcome index | Reason | By | At |
|---|---|---|---|
| 0 | Owner deferred deployment | Ada (`ada@example.com`) | 2026-10-11T09:30:00Z |
```

Rows are ordered by index, reference existing unchecked outcomes exactly once,
and use the existing text-cell and identity spelling/escaping. Done when remains
a checkbox list in descriptions and a yes/no table in task files. Exceptions
survive archiving and subsequent reads/writes; implicit compatibility settlement
never erases explicit unmet metadata. Older readers reject schema 4 and must
upgrade, not clear the sound file as set aside. Without exceptions (including
after restoring the last one), writers select the minimal applicable schema
1/2/3, independently of lifecycle Reason. This is a current-data encoding choice,
not a monotonic schema audit history. No live-board migration is required.
