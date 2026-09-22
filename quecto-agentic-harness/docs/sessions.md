# Sessions

Sessions persist conversation history so the agent remembers context across
prompts and restarts. They are the primary state mechanism for UDS agents.

## How sessions work

Each session is identified by a key. Two shapes exist today, and both are
carried unchanged by the typed `SessionIdentity` (the existing raw key, nothing
more — see [Identity](#identity-and-the-workspace-seam)):

| Kind | Key shape | Example | Created by |
|------|-----------|---------|------------|
| Named CLI session | `cli:<name>` | `cli:my-project`, `cli:default` | `quecto agent --mode uds -s my-project`; a one-shot `quecto agent -m …` without `-s` uses `default` |
| User chat | `chat-<unix-secs>-<uniq>` | `chat-1765930000-1a2b3c` | an unnamed `quecto agent --mode uds` run, and every `new_session` |

Sessions are stored as files in `<base_dir>/sessions/` by one flat layout
(`FlatSessionLayout`): `<sanitized key>.json` holds the conversation record
(a snapshot plus appended delta records with durable ordinals), `<sanitized
key>.owner` is the ownership stamp a live harness holds while it has the
session open, and `<sanitized key>/spill.jsonl` is the retained-context
namespace the `recall` tool reads. The file contains the full conversation
history (system prompt excluded — it is injected at run time and never
persisted; user messages, assistant responses, tool calls and results, the
workflow run and the historical sub-agent roster). For thinking-capable models
(Claude Sonnet 4.5+, Opus 4.5+), extended thinking blocks and their
cryptographic signatures are also persisted, enabling correct multi-turn
replay.

## Session modes

### Persistent (default)

```bash
# Unnamed UDS run: draws a fresh "chat-<unix-secs>-<uniq>" key
quecto agent --mode uds

# Uses session "cli:my-project"
quecto agent --mode uds -s my-project

# One-shot run without -s: uses session "cli:default"
quecto agent -m "hello"
```

When the agent starts, it claims the session key (a key another live harness
holds open is refused at startup) and, if a transcript exists, loads it —
provided its saved home admits the current execution directory (#2009, below).
All messages are appended to the session during the run. The session is saved
after each prompt completes, at every session transition, on an explicit
`persist_session`, and once more on the ordinary exit of the loop. A session
that exits with nothing to save leaves no transcript and no home sidecar.

**Legacy records are refused at startup, not claimed and loaded.** A transcript
saved before workspace scoping has no `.home`; because history is never
associated with a folder implicitly, `quecto agent -m hello` over a pre-existing
`cli:default`, or `-s <name>` over a pre-existing named session, exits 1 with
`session '<key>' cannot start here: it has no folder recorded (it was saved
before quecto tracked folders); start under a new name with -s <name>, or run
--no-session. The saved transcript was not changed.` The text names the way
out, and the old transcript is preserved untouched and stays
visible, with no folder recorded, in the All Folders list of `/resume`.
Associating such a session with a folder was descoped (#2045, #2014 not
planned): there is no installed base to migrate. The same wording covers
a session saved in another execution directory (start it from there, or start a
new name) and a home that is unavailable or whose workspace changed.

### Ephemeral

```bash
# No session loaded or saved
quecto agent --mode uds --no-session
quecto agent --mode uds -s -
```

The agent starts with an empty conversation. Nothing is persisted to disk, and
whatever the run spilled for in-run recall is scrubbed when the run ends.
Useful for one-off tasks or testing.

### Named sessions

Session names must contain only alphanumeric characters, hyphens, and
underscores:

- ✅ `my-project`, `feature_42`, `review2024`
- ❌ `../tmp/evil`, `my project`, `session@home`

## Session lifecycle in UDS mode

```
Agent starts
  │
  ├── Open the session: claim the key, load it from disk (unless ephemeral)
  │
  ├── Client sends prompt
  │     ├── User message added to history (and saved as a verified delta)
  │     ├── LLM response added to history
  │     ├── Tool calls/results added to history
  │     └── Session saved to disk (a clean delta, or a full record when needed)
  │
  ├── Client sends another prompt
  │     └── (same cycle, building on previous history)
  │
  ├── Client sends new_session / resume_session
  │     └── (resume: target claimed and loaded first, so a refused resume
  │         ends nothing, #2070) → children settled → departing session
  │         saved → roster reset → departing key released → the loop stands
  │         for the new identity (see the protocol reference)
  │
  ├── Last client disconnects
  │     └── Children torn down, session saved once more (ordinary exit)
  │
  └── Agent exits
        └── Socket file cleaned up
```

## Architecture (epic #1968)

The sessions capability lives under `src/application/sessions/` with the
shape the [target architecture](architecture/harness-architecture-map.md#persistence-and-session-recovery)
prescribes: use cases own every transaction and query, ports declare what the
capability needs, DTOs carry domain values only, and composition is the only
place a use case, a controller, a store or the active-session state is
constructed. The wire protocol these use cases answer is **unchanged** by the
epic — every command, field and refusal text is the same as before it
(see the [UDS Protocol Reference](uds-protocol.md), and the differential
probes recorded on each child PR of #1968).

### Use cases

Every `pub struct` under `src/application/sessions/use_cases/` is listed here;
the architecture tests refuse a use case the docs do not name.

| Use case | Catalogue | Triggered by | Owns |
|----------|-----------|--------------|------|
| `ListSessions` | #1861 | `list_sessions` | local/global discovery over saved summaries and authoritative home metadata, newest first |
| `SearchSessionMetadata` | #2010 | `search_session_metadata` | local/global search of saved-session metadata — title, exact opaque key, repository label, path — with scope, literal visible-text match, rank order, limit and advisory eligibility; never transcript content |
| `ReadHistory` | #1856 | `get_messages`, connect-time snapshot, child transcript forwarding | stable-id cursor selection and chronological paging of the live, published or persisted transcript |
| `RecoverMessage` | #1858 | `get_message` | full-copy recovery of a possibly collapsed message (ledger, then retention store), content ranges, tool-call arguments |
| `SynchronizeTranscript` | #1857 | `sync` (idle loop and busy reader) | epoch/revision reconciliation, reset-or-delta selection |
| `ExportSessionReport` | #1859 | `get_report` | latest eligible report selection, bounded preview, the optional raw export transaction and its admission |
| `SaveSession` | #1860 | `persist_session`, post-turn and pre-turn saves, transitions, ordinary exit | the one save transaction: prompt strip/re-inject, durable ordinals, dirty latch and watermark, restore reason, roster snapshot, full vs delta |
| `ClearConversation` | #1864 | `clear_history` | clear, ledger epoch, accounting reset, retention clear, save |
| `RewindConversation` | #1865 | `rewind_to` | target resolution, truncation, retention residue removal, ledger reset, accounting reset, save |
| `StartFreshConversation` | #1862 | `new_session` | settle children → save → reset roster → clear → fresh identity → release old key → propagate → switch → reset effort/workflow → clear retention |
| `DepartingChildren` | #1862/#1863 | (collaborator of the two transitions) | fleet settlement outcome and roster replacement policy (#1937, #1938) |
| `ResumeSavedSession` | #1863 | `resume_session`, and the startup open of the loop's own session | target admission, claim → load (another session is proven before the fleet goes, #2070; the loop's own key loads after its save) → settle → save → roster → release → propagate → effort → history/workflow restore → switch |
| `RecallContext` | #1866 | the `recall` tool; the run-end ephemeral scrub | recall/list/clear over the retention namespace of an identity |
| `RetainContext` | #1866 | the context-pruning policy | append with receipt, the `[:{k}]` deduplication rule |
| `ListRetainedContext` | #1866 | the context-pruning policy | the retention index and presence, never content |

Each transaction has exactly one owner: the handlers in
`src/interface/cli/uds_dispatch_session.rs` admit (the streaming refusal),
request the injected use case and present the response; they sequence nothing.

### Ports

Declared only under `src/application/sessions/ports.rs` and
`src/application/sessions/ports/`, each with a contract suite under
`tests/contracts/` that runs against the real adapter:

| Port | Adapter (production) | What it supplies |
|------|----------------------|------------------|
| `SessionHomeCatalogue` | `infrastructure/persistence/session_home_catalogue.rs` (`FileSessionHomeCatalogue`, its metadata query in `session_home_catalogue_metadata.rs`) | exact authoritative home reads, first-save home recording, derived catalogue validation and recovery, and the metadata query (#2010): every listed session once with its listing summary and home, validated by stamp; a record version already read — summarised, **or rejected** in this process (`session_home_catalogue_rejections.rs`: content verdicts only, in memory only) — is never read again |
| `WorkspaceDiscovery` | `infrastructure/workspace/git_scope_discovery.rs` (`GitScopeDiscovery`), using `filesystem_scope.rs` | canonical execution directory and real Git common-dir/worktree grouping; observable discovery failure |
| `SessionStore` | `infrastructure/persistence/session_store.rs` (`FileSessionStore`) | claim/release/load/save/save_delta/save_clean_delta/exists/list, keyed by `SessionIdentity` |
| `ContextSpillStore` | `infrastructure/persistence/context_spill.rs` (`FileContextSpillStore`) | append/recall/list_entries/has_entries/clear/scrub_sync of the retention namespace |
| `SessionExportPort` | `infrastructure/session_export.rs` (`FileSessionExport`) | the raw export writer (records, manifest, checksum) |
| `DurablePrefixObservation` | `application/durable_prefix.rs` (`DurablePrefixLatch`) | the agent loop's dirty-prefix latch the save drains |
| `WorkflowRunSource` | `infrastructure/persistence/session_snapshot_sources.rs` | the workflow run the save records |
| `HistoricalRosterSource` | `infrastructure/persistence/session_snapshot_sources.rs` | the sub-agent rows the save records as history |
| `TurnAccountingReset` | `interface/cli/uds_turn_accounting.rs`, `uds_session_switch_runtime.rs` | usage/context/pending reset when history is replaced |
| `FreshSessionIdentityGenerator` | `infrastructure/persistence/fresh_session_identity.rs` | wall clock + pid + counter → a fresh `chat-…` identity |
| `FleetSettlement` | `composition/fleet_settlement.rs` (adapts `TerminateAllDelegatedAgents`) | settle delegated children before a switch (#1938) |
| `DelegatedChildrenRoster` | `infrastructure/tools/delegated_roster.rs` | live delegated rows and roster replacement |
| `SessionKeyPropagation` | `interface/cli/uds_session_switch_runtime.rs` | the agent loop and its session-aware tools adopt the new identity |
| `SessionSwitchRuntime` | `interface/cli/uds_session_switch_runtime.rs` | effort reset, workflow reset/restore |

### Home authority and recovery (#2009)

`FileSessionStore` stores versioned optional home authority alongside each global
transcript as a `.home` sidecar. Ordinary full and delta transcript saves preserve
existing authority bytes, including unsupported or corrupt metadata. Only a new
persistent identity can receive its initial home; existing legacy records are not
automatically associated. Ephemeral runs write neither transcripts nor homes.

A home exists only with a transcript. The home is recorded before the first
transcript write (so no home failure can cost a transcript), and a save that
then commits nothing — the empty exit of a `-s` run or a TUI tab that never
spoke — removes the sidecar with the record it never wrote. A sidecar found
without a transcript at startup (a save that never committed, a transcript
removed by hand) is an orphan: it is discarded under the key's claim and the
session starts new, so a name is never locked to a directory with no history
and a stale home is never inherited by the first transcript written under the
key. A home beside a transcript is authority and is never touched by this rule.

The derived `home.catalogue` is discardable. Listing validates it against
authoritative records and rebuilds by atomic replacement: an unparseable or
version-incompatible index is recovery, reported once as `rebuilt` with a
diagnostic; an index that never existed (first use, a fresh install) is built
silently, and an index superseded by newer authority (a routine autosave) is
refreshed silently. A failed replacement returns valid discovered rows with
diagnostics, not a transcript rewrite. Orphan home files without committed
transcripts are not rows.

The index (`version` 2) holds no transcript content beyond each record's
listing title (its first user message, capped) and no content digests.
Per record, keyed by persisted key, it carries the transcript's file *stamp*
(device, inode, length, mode, mtime, ctime), the `.home` sidecar's stamp with
the decoded home observation, and the listing summary (title, message count)
the store's walk validated at that stamp. A process seeds its in-memory
projection and summary cache from a well-formed index once, then walks the
directory: one `stat` per file, and only a record whose stamp differs (or is
new) is read and strictly validated again — the same rule the in-process cache
always applied, so a transcript rewritten in place with its length and mtime
restored (new ctime) or replaced by a new inode is re-read. A cold process
over thousands of unchanged transcripts therefore lists in the time of the
walk, not of reading every transcript. The index is rewritten only after a
rebuild or when an entry changed. A record the strict validation **rejected**
is remembered by stamp too, so an unchanged corrupt record — a 100 MB append
cut short, say — costs one `stat` per query like a good one and still yields
its one file-named diagnostic on every answer; it is read again the moment its
stamp changes. Two rules bound that memory. **Only a verdict on content is
remembered**: the bytes were read in full and failed to parse or validate. An
I/O failure (no file descriptors, permission, a device error, a delete racing
the read) says nothing about the record, so neither the catalogue nor the
store's walk caches it: the record is named in that answer's diagnostics
(`<file>: session record unavailable: …`, or `<file>: session record not
listed: …` when only the walk missed it) and read again on the next query.
**And it is never persisted**: rejections live in memory, per process, and the
index carries none — so the derived index can never hide a session, mixed
harness versions cannot hand each other a verdict, and deleting
`home.catalogue` is always a complete recovery. The price is that each process
reads a corrupt record once (per validating half). An index that still carries
the `rejected` map one pre-release build wrote is read without it and
republished without it, silently. A summary seeds only beside its own file; a
doctored entry can at most misreport a home or title in the listing — it can
never remove a row — and
exact-key reads and resume admission read the `.home` sidecar, never the
index, so no entry can make a session resume-eligible. A store-listed record the strict catalogue has no row
for (a crash-truncated transcript mid-append) is not dropped: its `.home` is read
exactly, as admission reads it, and eligibility follows the domain rule, so the
user's most recent session stays Local and resumable after a crash and listing
never disagrees with exact-key admission; only an authority that cannot be read
at all is `Unavailable("record not in catalogue")`, with a diagnostic. Exact-key
admission reads authority independently of catalogue health.

The store's summary scan admits only regular `<sanitized key>.json` files whose
recorded key names that very file: a hand-renamed record, a symlink into the
sessions directory, an unreadable or invalid file, or a file replaced while it
was being read is skipped — deliberately, so no alias or foreign file can pose
as a session — and every skip is a `tracing::warn!` naming the path and the
reason, and is returned by the walk so `search_session_metadata` names the file
in its diagnostics, so nothing vanishes from the list silently. `list_sessions`
names every record the catalogue's validation rejected — nearly always the same
files — but a record only the walk failed to read (a transient I/O failure
between the two reads) is in the log alone there, as it always was: the listing
reads the store through the `SessionStore::list` port, which answers rows only.

**Git is a runtime dependency of scoped sessions.** Discovery runs the `git`
found on PATH (resolved once, spawned by absolute path off the async executor,
with the ambient `GIT_DIR`/`GIT_WORK_TREE`/`GIT_CEILING_DIRECTORIES` and config
cleared so only the requested directory defines the scope). Without a usable
`git` the adapter fails closed: discovery is observably unavailable, a new
identity is saved without a `.home` (legacy-unscoped) and no saved home admits,
so nothing is guessed as local or eligible. Discovery that fails for a new
identity (no Git on PATH, a mount boundary the parent walk stops at, an
unreadable current directory) never costs a transcript: the record is saved
without a `.home` — legacy-unscoped, visible in Global — and the failure is a
diagnostic.

Outside a repository the adapter also checks that no ancestor of the execution
directory carries a `.git` marker, up to (and not beyond) the filesystem the
directory lives on — the same boundary git's own parent walk stops at — so a
repository above a mount point (a bind-mounted subdirectory, a container volume)
does not turn "not a repository" into an unavailable discovery.

`SessionHomeContext` is an application observation collaborator, not another
query/save/restore owner. `ListSessions`, `SaveSession` and `ResumeSavedSession`
retain those responsibilities; it is composed once per loop over the one file
store (`composition/session_home.rs`) and is mandatory for resume, so admission
is never fail-open. The one eligibility rule is the domain's
`SessionHome::admission`: the saved authority re-observed unchanged at its own
directory, which must be the current execution directory in the same group —
`HomeChanged` when the directory's group changed since the save,
`DifferentExecutionDirectory` otherwise (a grouped worktree included). Resume
admission rechecks it after ownership admission for both explicit resume and
startup; a discovery group is not permission to execute history in another
directory.

#### Metadata search (#2010)

`SearchSessionMetadata` (`use_cases/search_session_metadata.rs`) is the one
owner of the picker's search. It consumes the `SessionHomeCatalogue` metadata
query and `WorkspaceDiscovery` through the loop's `SessionHomeContext`; it holds
no store. DTOs live in `dto/search_session_metadata.rs` (request with scope,
`QueryGeneration` and `SearchLimit`; `SessionMetadataRow` — a `ListedSession`
plus the repository label and the matched fields; `SearchFreshness`), and the
pure matching rules in `domain/session_metadata_search.rs`.

- **What is matched.** The listing title (the first user message, as the index
  holds it), the **exact opaque key**, the **repository label** and the
  **execution path** — literally; a term of at least three characters as typed
  that occurs nowhere literally may match the **title as an in-order
  subsequence** (`fxbg` ⊂ "fix bug"), the lowest tier, never the key, label or
  path (#2043, `domain/session_title_subsequence.rs`). Nothing else exists in the query's input, so transcript
  content can never match: no transcript is ever read to MATCH. What is read
  is decided by freshness alone — the adapter joins the store's summary walk
  with the validated home listing, both stamp-checked and index-seeded, so a
  record version that was already read costs one `stat` per half and is not
  opened again, whether it was summarised or — in this process — **rejected**
  (a corrupt or cut-short record is remembered by stamp with its diagnostic, in
  memory only: a new process reads it once; a failed READ is never remembered).
  A new or changed record is read once by each half, and with the
  index absent, unreadable or version-incompatible every record is — once per
  half, i.e. twice in all (the two halves validate differently; sharing the
  read is follow-up work, #2042; see *Performance*). `tests/contracts/
  session_metadata_search.rs` counts zero transcript reads over 2,000 valid
  records plus an unparseable one and a 2 MiB cut-short one, warm, and from a
  new process exactly those two rejected records once per half;
  `session_rejection_cache.rs` pins re-reading on change, that a transient read
  failure is retried and leaves no trace, and that a persisted rejection — even
  at the correct stamp of a valid record — hides nothing.
- **How.** Literal text, never a pattern: regex, glob, SQL and shell
  metacharacters are ordinary characters. Query and fields are compared as
  *visible text* — control characters and invisible format characters (bidi
  controls, zero-width characters, the soft hyphen, tags, the BOM) are dropped
  from both sides, whitespace runs collapsed, and case **folded**
  (`domain/session_metadata_text.rs`): the whole text is Unicode lower-cased,
  then final sigma is a sigma (`οδος` finds `ΟΔΟΣ`), sharp s is `ss` (`straße`
  finds `STRASSE`), a dotless `ı` is an `i`, and the combining dot a dotted
  capital `İ` lower-cases into is dropped after an `i` (`istanbul` finds
  `İstanbul`) — so a hidden character can neither hide a record nor forge a
  match. Two limits remain: code points are compared as stored (the harness
  ships no normalization tables: a decomposed `é` is not a composed one), and
  ligatures and other full-fold expansions (`ﬁ`) are not expanded. Whitespace separates terms; **every**
  term must occur in the title, the label or the path. A key matches only when
  the whole trimmed query equals it byte for byte — no fragment, no case fold.
  A path that is not UTF-8 is matched and presented with each byte that is no
  text spelled `\xNN` (`caf\xE9`) and a literal backslash doubled
  (`domain/session_path_text.rs`: a folder really named `caf\xE9` is
  `caf\\xE9`), so the spelling is injective and two folders stay
  distinguishable by `executionPath` and `repositoryLabel` alone. A query of
  more than 256 visible characters — counted before the fold, so `ß` is one —
  is refused whole (`refused`, `domain/session_query_refusal.rs`), never
  searched as a prefix; the answer echoes the visible text that was searched;
  a query with nothing visible names every session in scope.
- **Repository label.** For a Git home, the directory that holds the `.git`
  common dir (so linked worktrees share their repository's label) or a bare
  repository's name without `.git`; for a folder home, the folder's name.
  Legacy unscoped and unavailable homes have no label and no path: they are
  found by title and key, and presented with `homeState` so a client labels
  them explicitly.
- **Scope, order, limit.** `global` is every listed session; `local` is the
  current workspace group and, without current workspace facts, nothing — never
  everything. In `local` every row shares the group's root and its repository
  label, so neither can tell two rows apart and any word occurring in them
  would match every row: a Local search matches the title, the exact key, and
  the path **below the group root** (for a linked worktree outside the root,
  below their common ancestor) — a sub-folder's or a worktree's own name still
  finds its row, reported as `path`; the repository label is not matched. Rows are ordered by the best matched field (key, title,
  repository, path), then newest first (undated last), then key: a total order.
  At most `limit` rows (default 200, 1–500) are returned, with `totalMatches`
  and `truncated`.
- **Freshness.** Every search validates against authority exactly as a listing
  does: same diagnostics (a corrupt record and a `.home` that needs repair are
  each named by file, once per query; siblings stay), same
  recovery (`rebuilt` with a diagnostic for an unparseable index, silent refresh
  for a superseded one), and a store-listed record the strict catalogue rejects
  keeps the home admission would read. Exact-key resume never depends on it.
- **Selection.** A row is a listing row and carries the same identity-bound
  `homeVersion` (the contract suite asserts search row = list row = authority).
  Selecting one goes to `ResumeSavedSession` like any other row: a session
  deleted since is `not_found`, a re-homed one `stale_home_version`, a
  cross-folder one the typed `belongs_elsewhere` refusal. Eligibility in a row is advisory, decided
  by the one domain rule over the single discovery of the current directory the
  query makes (a home saved anywhere else can never admit, so nothing is
  discovered per row).
- **Generation.** `generation` is the client's own counter, echoed unchanged;
  the application attaches no meaning to it. The TUI sends at most one search at
  a time per picker (an edit made meanwhile goes out when the answer arrives —
  latest wins). Only the answer that carries the id that was sent **and** the
  latest generation *settles* the picker — Enter and the mouse act on settled
  rows only, and an Enter typed ahead is applied to the settled answer's top
  row. An overtaken answer is shown as progress (never across a scope change
  or a cleared box); a search unanswered for 5 s is re-issued once, then given
  up. Typing faster than a round trip costs two searches per burst; typing
  slower costs one per key.
- **Performance.** Every query stamps every record (no time-limited cache, no
  directory-mtime short-circuit — transcripts are appended in place, which
  does not touch the directory). On a 5,201-record store a warm search takes
  ~90 ms against a 50 ms target: **missed, ~1.8×**. The cost is the two `stat`
  passes (store walk + catalogue scan); sharing one pass, or stamping in
  parallel, is tracked as follow-up work (#2042). Commands are dispatched FIFO, as
  `list_sessions` is: a client that queues searches back-to-back delays its own
  `get_state`/`abort` by the sum — the TUI's single flight bounds that to two
  scans; a raw client gets no such bound.

#### A session that belongs elsewhere is refused (#2011, #2045)

`ResumeSavedSession::execute` takes a `ResumeRequest` (exact target, optional
expected `HomeVersion`) and answers `ResumeOutcome::Resumed` or a typed
`ResumeSavedSessionError`, each with a stable `code()`. A target that exists
but does not admit a restore here is `Decision(ResumeDecision)`: the
`ResumeDecisionKind` (`CrossFolder`, `HomeMissing`, `HomeChanged`,
`HomeUnknown`, `LegacyUnscoped`), the home version it was judged on, the
recorded folder and the adapter's detail. Each kind has its own refusal code
(`ResumeDecisionKind::refusal_code`: `belongs_elsewhere`, `home_missing`,
`home_changed`, `home_unknown`, `no_home_recorded`). **Nothing is offered and
nothing follows**: the descope of #2045 removed the resume actions (Open
original, Fork, Locate, Associate, Cancel), their capability set, their
composition and their refusal codes — quecto is a lightweight harness, and a
session from another folder is resumed by opening quecto there. The client is
told how: `domain/session_open_command.rs` spells `cd '<folder>' && quecto-tui`
from the REAL folder as one POSIX single-quoted word (a contract test runs it
in `sh` against hostile names), or spells **no command** when it would not
read the way it runs — a folder that is not UTF-8, or one holding a control,
Bidi_Control or invisible format character. `quecto-tui` takes no session
flag, so the second step travels as its own field (`/resume <key>`). Both are
sent for ONE kind only — `ResumeDecisionKind::resumes_by_opening_quecto_there`
(cross-folder): a changed home may be this very folder, a missing one cannot
be entered, and an unknown or unrecorded one names nowhere to go, so those
refusals carry neither. The startup refusal is read by someone who ran
`quecto … -s <key>`: under the same rule, only a session that lives in another
folder is given the folder, `cd '<folder>'`, and "then run the same command
again"; the others say why and that the saved transcript was not changed.

A request that still names an `action` (a pre-#2045 client) is
`ResumeSavedSessionError::LegacyAction` (`legacy_action_unsupported`): the
wire field is presence-sensitive — any value, even `null` — so such a request
is refused under its own id through the ordinary refusal path, never a
`parse_error` and never read as a restore.

The interface's idle gate is the typed refusal `Busy` — a defensive guard:
turns and dispatch share one task, so a mid-turn request is queued and
**executes after the turn**, and the guard is unreachable today.
`SessionHomeContext::classify` is the one classification (`admit`, the startup
disposition, is derived from it); an undiscoverable current directory is the
refusal `CurrentScopeUnavailable`. The collaborators in
`use_cases/resume_saved_session_decision.rs` run the same `decide` twice: an
effect-free pre-flight — it asks `SessionStore::exists` (no transcript is
read) and answers an absent target `NotFound`, a session that belongs
elsewhere or a stale selection at once, so none of them settles a child, saves
or claims anything (the loop's own key is excepted from the early `NotFound`:
the departing save may be what first writes it) — and the authoritative
re-check under the target's claim, where a changed `HomeVersion` is
`StaleHomeVersion` and the claim is released (the loop's own key excepted,
#1995). Exact-key resolution reads the `.home` authority and never the derived
index; an exact miss never resolves a prefix or fuzzy neighbour.
`HomeVersion::of(identity, scope)` is a pure, process-stable digest of the
session identity and the authoritative scope (legacy and uninterpretable
states included — the latter by category, not by error wording): two sessions
never share a token. A `list_sessions` row carries
`ListedSession::home_version()`, the same function over the listed home; the
contract `resume_decision_listing.rs` pins row version == authority version
for every home state, cold and index-seeded, and that a well-formed lying
index yields only `StaleHomeVersion`; `resume_refusal.rs` pins every kind, the
no-effect invariants and the single ownership over the real adapters. The UDS edge is `interface/uds/sessions/
resume_session_controller.rs` (wire fields → request) and
`interface/cli/uds_dispatch_resume.rs` (typed answers, untrusted text made
safe); the TUI shows the refusal as a plain notice
(`sessions/resume_decision.rs`) that sends nothing, and a selection sends
identity + the listed version — never an action.

### DTOs and the active session

`src/application/sessions/dto/` carries requests, results and errors as domain
values (messages, ids, identities, ledger positions); no `serde_json::Value`,
no `AgentEvent`, no transport type. Every refusal text a client can see is a
DTO `Display` impl. The one live-conversation read model is the
application-owned `ActiveSessionState` (`active_session.rs`: typed identity,
the `ConversationLedger` published transcript with its bounded full-copy
ledger and sync frontier, the retention backstop, and the persistence state —
watermark, sticky durable-prefix latch, killing exit). It is created once per
loop by composition; the interface publishes into it and reads through the
use cases. The `sessionKey` every presenter reports (`get_state`,
`get_session_stats`) is read from this identity — no interface tracker holds
a copy (#1979).

### Composition

- `src/composition/sessions.rs` — `build_session_handles` (the `FileSessionStore`
  over the one `FlatSessionLayout`, `ListSessions` and its controller, the
  fresh-identity generator) and `build_retention_handles` (the
  `FileContextSpillStore` over the same layout, and the recall/retain/list graph).
- `src/composition/active_session.rs` — the per-loop graph over the
  `ActiveSessionState`: history, recovery, sync, save, clear, rewind, fresh,
  resume and their controllers.
- `src/composition/session_report.rs` — the export root
  (`<base>/artifacts/session-exports`), `FileSessionExport`, `ExportSessionReport`.
- `src/composition/retention.rs` — `RecallContext`, `RetainContext`,
  `ListRetainedContext` over one store.
- `src/composition/fleet_settlement.rs` — the fleet adaptation of the switch.

The interface holds plain handle structs (`src/interface/cli/uds_session_handles.rs`,
`retention_handles.rs`) filled through `CliComposition`; it names neither the
composition layer nor an adapter, converts no raw key, and reaches no store
method — the architecture tests under `tests/architecture/sessions_*.rs` pin
every one of these rules with exact, decrease-only inventories.

### Retained context: who owns what

Sessions is the single owner of durable retained context — the
`ContextSpillStore` port, the identity-keyed recall/list/clear selection, the
id append and deduplication. The context-pruning policy
(`src/application/context*.rs`, the agent loop) decides *when and what* to
retain and consumes the narrow `RetainContext`/`ListRetainedContext` handles,
never a store method; it also allocates the spill ids (`turn{n}:{tool}:{idx}`,
`turn{n}:msg:{role}`). The `recall` tool is an adapter over `RecallContext`:
schema parse, result formatting, diagnostics.

One raw conversion remains by design: the tools capability's
`Tool::set_session_key(String)` port is raw, so `RecallTool` converts the key
it is handed with `SessionIdentity::from_persisted_key` — the one admitted
infrastructure conversion site, pinned exactly by the architecture tests.
Typing that port is a tools-capability change and future work outside this
epic. The tool registry's startup key (`Session::build_key("cli", name)` in
`agent_tool_registry.rs`) is the same capability's raw input and is admitted
on the same terms.

### Identity and the workspace seam

`SessionIdentity` (`src/domain/session_identity.rs`) is the **existing raw key
only**: `ephemeral()`, `named_cli(name)`, `user_chat(key)`, `fresh_chat(secs,
uniq)`, and the total persistence round-trip `from_persisted_key`. It has no
scope field, no variant, no path conversion and no workspace behaviour, and
this epic adds none.

Folder-aware discovery (#2009, parent #2001) keeps `SessionIdentity` opaque
and retains `FlatSessionLayout` and the global transcript store. Home metadata
is separate from identity: repository/worktree grouping controls discovery,
not the permission to restore history in a different execution directory.
Canonical exact-folder identity applies outside Git; the nearest repository
wins inside Git. Legacy records without home metadata remain unassociated.
Invalid or unsupported home metadata is unavailable, never legacy-unscoped.

The first discovery slice excludes metadata search and executable cross-folder
actions. Open-original, fork, locate and first association belong to later
children of #2001; discovery must never silently substitute history reuse for
an unavailable action.

## Context management

As conversations grow, the agent manages context automatically:

### Context window

The agent tracks estimated token usage against an application-level context
budget. When the conversation exceeds `max_context_tokens` (configurable,
default `200000`), the agent applies context pruning:

1. **Spilling at creation**: Tool outputs *and* conversation (user/assistant)
   messages are written to the session's retention namespace when they are
   created, so anything later collapsed or dropped can still be recovered with
   the `recall` tool
2. **Tool output collapsing**: Once the session accumulates more than
   `context_collapse_after_tool_calls` tool calls, the oldest tool outputs are
   replaced with compact recall stubs. The trigger counts tool calls
   cumulatively across prompts within a session. Current config default: `50`.
   Set it to `4294967295` (`u32::MAX`) to disable collapse.
3. **Conversation message collapsing**: An independent dial,
   `context_collapse_after_messages`, keeps the most recent N conversation
   messages in full and replaces older ones with one-line recall stubs.
   Exempt: the system prompt, spill manifest, in-flight user prompt, and the
   `pin_recent_turns` most recent turns. Defaults to 50 (mirroring the
   tool-call collapse default); set to `4294967295` (`u32::MAX`) to disable.
4. **Demotion ladder**: When the conversation still exceeds the effective
   budget, messages are demoted down a ladder — full content is collapsed to
   recall stubs first (oldest first), and only if the budget is still
   exceeded are stubs removed entirely (their content stays on disk). Pinned
   and tail-pinned (`pin_recent_turns`, default `2`) content is never
   demoted; if the pinned set alone exceeds the budget, a
   `context_prune` warning is logged and the `ContextPruned` audit event
   records `budget_unmet`.

The effective budget is the smaller of `max_context_tokens` and the active
model's context window when the model registry declares one.

### Spill and recall

Retained content is appended to `<base_dir>/sessions/<sanitized key>/spill.jsonl`
(the session's retention namespace; `new_session`, `resume_session`,
`clear_history` and `rewind_to` move or clear it with the conversation). Once
the namespace is non-empty, the conversation carries a pinned, static guidance
message that points the model to `recall("list")`; it deliberately contains no
spill count, IDs, or previews, so the provider-visible prompt prefix remains
byte-identical as the spill set grows. `recall("list")` returns the complete
live index on demand, and the `recall` tool description advertises that route.

When a message is collapsed, the compact stub looks like:

```
[bash: ls -la (2450 tokens) — recall("turn5:bash:0")]
```

The agent can call `recall("turn5:bash:0")` to retrieve the original content
from the retention namespace, even after the original message has been
collapsed or dropped by the sliding window.

## Inspecting sessions

### Via UDS commands

```json
{"type": "get_state"}
```

Returns the slim supervision projection: the session key, generation,
streaming state, model, effort and workflow identity.

```json
{"type": "get_messages"}
```

Returns the newest bounded page of conversation history (#1061).

```json
{"type": "get_messages", "count": 5}
```

Returns the last 5 messages (omit `count` for the newest bounded history page; the response's `before`/`hasMoreBefore` fields page older history, #1061).

```json
{"type": "get_session_stats"}
```

Returns the session key, token usage, message counts, and cost estimates.

### Clearing history

```json
{"type": "clear_history"}
```

Clears all messages except the system prompt. Drains any pending follow-up
or steer messages. Fails if the agent is currently streaming. See
UDS Protocol Reference (`docs {"name":"uds-protocol"}`) for full details.

## Configuration

Session behavior is configured in `config.json` under `agents.defaults`:

```json
{
  "agents": {
    "defaults": {
      "max_context_tokens": 100000,
      "context_collapse_after_tool_calls": 3
    }
  }
}
```

| Field | Default | Description |
|-------|---------|-------------|
| `max_context_tokens` | `200000` | Application-level token budget before context pruning (clamped down to the model's declared context window when known) |
| `context_collapse_after_tool_calls` | `50` | Collapse the oldest tool outputs once the session exceeds N tool calls. Set to `4294967295` (`u32::MAX`) to disable |
| `context_collapse_after_messages` | `50` | Collapse the oldest conversation (user/assistant) messages to recall stubs once the session exceeds N live messages. Set to `4294967295` (`u32::MAX`) to disable |
| `pin_recent_turns` | `2` | How many most-recent turns the context ceiling never demotes or drops |

## See also

- UDS Protocol Reference (`docs {"name":"uds-protocol"}`) — `list_sessions`, `search_session_metadata`, `resume_session`, `new_session`, `persist_session`, `get_state`, `get_messages`, `get_message`, `sync`, `get_report`, `get_session_stats`, `clear_history`, `rewind_to`
- Harness architecture map (`docs/architecture/harness-architecture-map.md`) — the sessions capability in the layer model
- Subagents (`docs {"name":"subagents"}`) — each subagent gets its own session
