# Board server (`quecto board-server`)

`quecto board-server serve --socket <path> --repo <clone> [--remote origin]
[--listen <ip:port>]... [--state <dir>]` serves a repository's project
board over HTTP/1.1 on a Unix socket, and on each allowlisted TCP address,
until SIGINT or SIGTERM (epic #2705 B4, B5, B7).

The board is the server's own: a SQLite file, `board.sqlite3`, in the state
directory (below). The clone gives only the board's id (from its remote's
URL) and, for the board's operations, its remote's revisions and pull
requests. A new server has no board until a `bootstrap` op makes one (an
import of a `quecto/board` branch, `board-server import` below, writes
this same file; `export` and the backup read it). A `board.sqlite3` of another
schema version (an earlier build's) is refused at start, naming the version
found and the one this build writes: move the file aside and start the
board again (there are no boards to keep yet).

## Who may connect

- **The Unix socket.** Only a process of the server's own user may
  connect, checked by peer credentials. No token is needed. The socket is
  mode 0600 in a directory of mode 0700. Peer credentials apply to the
  Unix socket only.
- **TCP.** The server listens only on loopback (`127.0.0.0/8`, `::1`) or a
  Tailscale address (`100.64.0.0/10`, `fd7a:115c:a1e0::/48`). Any other
  address, `0.0.0.0` and `::` among them, is refused at start. Every TCP
  request needs `Authorization: Bearer <token>`. Without one, or with an
  unknown or revoked token, the answer is 401 whatever the method, path
  or body: the token is checked before anything else about the request,
  and before its body is read, and the 401 closes the connection. The
  admin routes answer 403 over TCP, whatever the token. Over TCP a token
  store failure is answered only `the token store is unavailable`. The
  detail is said on the Unix socket, and goes to the server's stderr
  only when it changes.
- **Addresses, not interfaces.** `100.64.0.0/10` is matched by address,
  not by interface: an address in that range on a non-Tailscale interface
  would be allowed too. A token is still required on every TCP request.

The addresses come from `--listen` (repeatable), else from the global
config (`~/.quecto/config.json`, or `--config`). The `board_server`
section is global-only: a repository overlay may not set it.

```json
{ "board_server": { "listen": ["100.101.102.103:7979"] } }
```

## Device tokens

Each device that reaches the server over TCP holds its own token.
Administer them over the Unix socket, while the server runs:

| Command | Does |
|---|---|
| `quecto board-server token add <name> --socket <path>` | makes a token and prints it once, alone on stdout |
| `quecto board-server token revoke <name> --socket <path>` | revokes it; it is refused from the next request |
| `quecto board-server token list --socket <path>` | names and created dates, never a token |

A name is 1 to 64 of `a-z`, `0-9` and inner `-`. A token is 32 random
bytes, base64url. Only its SHA-256 is kept, in `tokens.json` (mode 0600)
in the state directory (mode 0700): `--state <dir>`, else
`~/.quecto/board-server/<board id>/` (a `--state` ending in `/` is
refused). The server reads the file once, at start, verifies tokens from
memory in constant time, and writes each change through: a temporary
file renamed over it, both relative to the locked directory. A server
holds at most 256 tokens. The server holds an exclusive
lock on the state directory itself for its whole life, so a second
server on the same state directory (say, a second clone of the same
remote) is refused at start; give it its own `--state <dir>`. If the
directory is moved away and replaced while a server runs, that server
refuses every token change. Edit `tokens.json` by hand only while the
server is stopped: the server reads it once, at start, and refuses to
overwrite a file changed behind it (503 `unavailable`, saying to restart
the server to pick up the edit). (After B2 the tokens move into the
SQLite store as admin revisions.)

## Routes

| Route | Answer |
|---|---|
| `GET /v1/status` | `{"role": "primary", "board": "<id>", "head": "<head>" or null}` (read from the server's own board file, never from the clone's remote, so a slow or silent remote never delays it) |
| `GET /v1/time` | `{"time": "<RFC 3339, to the second>"}` (the server's clock; 503 `clock_unsynced` while it is not synchronised) |
| `POST /v1/boards/{id}/op` | one board operation (below): `{"answer": {..}, "facts": {..} or null}` |
| `GET /v1/boards/{id}/events` | the change feed, a `text/event-stream` (below) |
| `GET /v1/boards/{id}/revisions?after=<n>[&limit=<1..500>]` | `{"revisions": [<revision events>], "epoch", "tip", "more"}` (below) |
| `GET /v1/admin/tokens` | `{"tokens": [{"name": "<name>", "created": "<RFC 3339>"}]}` (Unix socket only) |
| `POST /v1/admin/tokens` `{"name": "<name>"}` | 201 `{"name", "created", "token"}`: the one time the token is shown (Unix socket only) |
| `POST /v1/admin/tokens/revoke` `{"name": "<name>"}` | `{"name": "<name>", "revoked": true}` (Unix socket only) |

The board snapshot route comes with B11.

## Operations (`POST /v1/boards/{id}/op`)

`{id}` must be this server's board id (16 lowercase hex digits), or the
answer is 404 `board`. The body is one JSON object, read strictly (a key
named twice anywhere is refused) and within the front ends' 1 MiB request
bound:

```json
{"request": {"op": "create", "task": "a", "title": "A", "status": "ready"},
 "front_end": "cli",
 "summary": false,
 "noted": false,
 "write_id": "6f1c0a52-7d0e-4c55-9a51-2b8f3a3e8c10",
 "committer": {"name": "quecto pocket", "email": "pocket@quecto.invalid"}}
```

- `request` is the request exactly as `quecto board --input` or the agent
  `board` tool takes it, and `front_end` (`cli` or `tool`) is the front end
  that read it. The server reads it again as that front end does, runs it,
  and `answer` is that front end's answer to it: what the board on this
  machine would answer. A request the board refuses (an unknown op, a bad
  field, a task not on the board, a task that exists) is still 200, its
  refusal in `answer`. A failure of the store itself (`store_unavailable`,
  `clock_unreadable`) is 503 `unavailable` instead: a write is rolled back
  and keeps no answer, so its retry runs it again. So is the clock gate's
  refusal, as 503 `clock_unsynced` with the host's `reason` (see below).
- `front_end` is the client's to choose, so a device token may send
  `"cli"` and have its request read by the CLI's entry policy (which still
  takes a legacy free-text description). This is accepted: tokens are for
  trusted devices.
- `summary` asks a listing for its tasks' summaries (the CLI's
  `--summary`); `noted` asks for `facts`: `{"attempts", "lost", "head"}`,
  the write's attempts, the races it lost and the revision it made.
- A write needs `write_id`, a lowercase UUID (`8-4-4-4-12` hex digits;
  the client makes a UUIDv4), and `committer`, the identity the caller
  asserts. A read ignores both.
- **Identities are self-asserted** (an owner decision): the server checks
  only that the committer is one a task can hold. A device token may
  assert any identity, its own or another's; every revision records the
  principal the server let in beside the identity asserted, so who
  actually wrote is always on record. Allowlisting the identities each
  token may assert is future work.

A write runs under the board's write lock in one `BEGIN IMMEDIATE`
transaction, so its compare-and-swap never loses, and every write it makes
(a change and the completion it triggers) lands together or not at all.
Each revision records the committer as its author and, beside it, who the
server let in: `local` on the Unix socket, `token:<name>` over TCP. The
answer is kept, as the write commits, under its principal and `write_id`,
with a hash of its front end, request text and committer. The same
`write_id` from the same principal for the same write is answered with
that first answer, byte for byte, and writes nothing; for any other write
it is 409 `write_id_reused`. Another principal's `write_id` is its own, so
a token can never replay a local write's answer. The last 1,024 answers
are kept: a client retries within seconds. A read takes no lock and no
transaction.

A write is admitted first: two are held at once (one running, one waiting
on the lock), and a third, or a retry of a write whose first run is still
running (by principal and `write_id`), is answered 503 `busy` at once,
before any host is read. So waiting writes never take the blocking threads
reads need, and a busy answer costs no `gh` call. An admitted write whose
answer is already kept (a retry after its first run landed) is answered
from it at once, again with no host read; the transaction still judges a
write id it does not find. Otherwise an admitted write reads every host it may need (the pull requests' states, for completing
its task: none for a task not on the board or linking none) before the write
lock, so the transaction touches only the store and a slow `gh` holds up no
other write. Those reads share the 30 s budget a local completion has: a
pull request left unread past it is unavailable, so the write is answered
(`pull_requests_unavailable`) rather than failed. A write's worst case is
that budget plus one 20 s `gh` read, then its turn at the lock behind one
other write, which stays under the client's 75 s reply timeout.

**Runs across the wire (B8).** A run's swarm runs where its client's
session is, so the server and the client each do their half:

- **start**: the server begins it (the task started and run 1 of its
  accepted plan written in one change) and answers what its coordinator
  is launched with: the start's answer without `coordinator`, plus the
  status it took the task `from` and `launch` (`plan_version`, the
  coordinator's `brief`, `model`, `effort`). The client launches the
  coordinator and answers the start with its `coordinator` in place of
  those two. A launch that fails, or a call cancelled during it, is undone
  on the server: the client sends the same start with
  `"undo": {"run", "from", "reason"}`, the server folds the run back as
  failed (as a local undo does) and answers `launch_failed` with
  `run_state`; the client answers the start's `launch_failed` from it. An
  undo the server never answers leaves the run `open`, for `end_run`; one
  is never sent while the client sees the coordinator running.
- **end_run**: the client sends what its session saw beside the request,
  `"facts": {"liveness", "reports"}`: `liveness` is `"unseen"` (a front
  end that sees no swarm, the CLI) or `{"seen": <agent or null>}`, and
  `reports` is `null` or the swarm boards it kept for the run
  (`{"ended", "left"}`). The server judges the end on them inside its
  transaction: `run_live` while the client saw the coordinator running,
  the kept board's items, owners, evidence and cost. A success's evidence
  revisions are read from the server's own remote before the write lock,
  on a thread of their own within the 30 s budget; past it, the success is
  refused as for a remote that cannot be read. Nothing inside the lock
  asks git or a front end. A client drops a kept board once the server
  recorded it.
- **left runs**: a session owes each run its starts launched that the
  server's `show` gives as still running.

The facts are trusted cooperatively: a client with write access asserts
what its front end saw, as a local board trusts its own front end. A
client could end a run it says is not running; the server cannot see the
client's swarms. The remote's revisions and the pull requests' states the
server reads itself. A write's `write_id` is bound to its facts or undo
too.

The facts are a strict wire contract: the swarm boards' fields (`end`,
`items` with `title`, `state`, `owner`, `evidence` and
`stood_before_death`, `observed_tokens`) are read with unknown fields
refused, so a client and server of different builds can refuse each
other's ends. Upgrade every host that runs a board server or a client of
one together. Facts past the request's 1 MiB go smaller: first without the
board a run was left on beside the one it ended on; an end that is no
success then without its evidence, then without its boards; a success
keeps only the evidence naming commits, and is refused on the client
("the run's facts are too large to send") if still too large. An undo of
an open run is made only by the holder of the task's live claim, while
the task is in progress and the run is its latest, from `ready`,
`claimed`, `in_progress`, `review` or `blocked`; a refused undo is
answered in the start's `launch_failed` with the server's code and words
(`run_state` `refused`). A lost begin or undo is sent once more, the same.
The server reads the remote for a success's plan items' evidence alone,
refusing one naming more than 2,048 commits, one read at a time: a success
while one runs is answered 503 `busy` ("the remote is being read for another write: try again shortly"), nothing written. A `409 standby` from a follower and its epoch come with B12.

**The clock gate.** The server's clock tells the time only while the
host affirms its clock is synchronised. On Linux the server asks the
kernel (`adjtimex`, read-only) once per op, as the op begins, and holds
that word for the op: the clock is synchronised only when `STA_UNSYNC` is
clear and the kernel's state is one it keeps while synchronised
(`TIME_OK`, or a leap second pending or passing). Anything else
(`STA_UNSYNC`, `TIME_ERROR`, a failed call, a state this build does not
know, or any other platform) is not.
- Synchronised: the server's time is the host's. It is still no outside
  reference, so an answer to a request that reads the clock keeps its
  `"clock": {"referenced": false, ...}` note, and the burst bound
  (`task_ahead`) stays off on the server until B9, as in B7.
- Not synchronised: a request that reads the clock (`create`,
  `update_draft`, a change, a plan op, `complete`, `done_without_run`,
  `end_run`, `list_takeable`) is refused 503 `clock_unsynced`,
  `{"error", "code", "reason"}`, rolled back and kept by no answer, so the
  same `write_id` runs again once the clock is synchronised. A client
  answers its caller `clock_unsynced` with the reason; stop and report it.
  Requests that read no clock (`show`, `list`, `list_ready`, `bootstrap`,
  `clear_set_aside`) are served as ever, and `GET /v1/time` is refused
  alike.
- Not gated: the board's own clock reads only. Tokens are dated by the
  host's clock as it is (a token's date bounds nothing), and the SQLite
  store stamps each revision with the host's time as it is.
- What the gate reads is the kernel's sync flag, not the daemon itself.
  systemd-timesyncd and ntpd clear `STA_UNSYNC` once they are in step;
  chrony clears it only with `rtcsync` (on by default in most distro
  configs), so a synchronised chrony host without it is refused. Check a
  host with `timedatectl show -p NTPSynchronized` (`yes` is what the gate
  needs).
- The kernel sets `STA_UNSYNC` by itself only once its `maxerror` passes
  16 s, so if the time daemon dies without clearing sync, the gate can
  still say synchronised for hours (16 s at 500 ppm is about 9 h). Setting
  the time by hand (`date -s`, `settimeofday`) sets the flag at once.
- A board on a clone's own branch has no gate: it reads the remote's time
  as before.

## Clients

`quecto board ... --url <url>` sends the request to a board server. With
no `--url`, `QUECTO_BOARD_URL` names one, else the global config's
`board.url` (the `board` section is global-only); with none, the board is
the clone's own branch, as before. The agent `board` tool follows
`QUECTO_BOARD_URL` and `board.url` too.

- `unix:///<socket path>`: the server's own user, no token.
- `http://<ip>:<port>`, on loopback or a Tailscale address: the token is
  read from `QUECTO_BOARD_TOKEN`, else `board.token`, never from the
  command line, and sent as `Authorization: Bearer`. No error, answer or
  log shows it.

```json
{ "board": { "url": "http://100.101.102.103:7979", "token": "<token>" } }
```

The client sends one request per operation and decodes no board: a write
is one round trip. It reads no clock (the server's is the board's one
clock). The board's id comes from `--board-id`, else from the clone's
remote as for a local board. A write is made as `QUECTO_BOARD_COMMITTER`
(`name <email>`) when it is set, else as the clone's configured committer.
A TCP connect may take 5 s; a read's reply 30 s, a write's 75 s. After a
transport failure before a reply (no connection, a connection dropped
or closed unanswered) the client sends the same body again, with the same
`write_id`, up to 3 times. A reply that never came is not sent again: a
read is answered `store_unavailable`, and a write `unanswered`, saying it
may have landed, so read the board before making it again (a raw client
may retry the same `write_id`, and gets the kept answer once it lands). After a `503 busy` (a token past its 4 answers or 8 connections at once, or the
server past its 2 writes) it sends it again after a jittered pause of
50 to 500 ms, for up to 10 s of pauses in all; still busy, it answers
`busy`, a code a caller may retry. Any other failure is answered
`store_unavailable`, naming the server's status and code.

## Change feed (B10)

The feed tells which revisions the board made, in order, by number and
kind alone. No event carries a task's text, a message, an identity or a
principal: a reader that wants the change reads the board. Every event's
data is one JSON object of this allowlisted schema, and nothing else:

| `kind` | data | SSE `id` |
|---|---|---|
| `revision` | `{"kind": "revision", "epoch": <n>, "revision": <n>}` | the revision |
| `epoch` | `{"kind": "epoch", "epoch": <n>}`: the epoch the next revisions are in | the revision before them |
| `reset` | `{"kind": "reset", "after": <n>}`: the stream ends; read on from `after` | `after` |

Revision numbers are unique and increase across epochs. An `epoch` event
comes before the first revision of a stream and before any revision whose
epoch differs from the one before it; a stream opened at the tip starts
with the current epoch (its id the tip, so a reconnect misses nothing).
An epoch opened with no revision (B11/B12) is told with the next revision.

**`GET /v1/boards/{id}/events`** answers `200`, `Content-Type:
text/event-stream`, `Cache-Control: no-store`, `Connection: close`, a
chunked body. It starts from the `Last-Event-ID` header (what an
`EventSource` sends when it reconnects), else `?after=<revision>`, else the
tip (live only). Each event is `id: <n>`, `event: <kind>`, `data: <json>`;
the stream opens with `retry: 1000`, and an idle stream sends a `: ping`
comment every 15 s. Resume is gap-free with each revision exactly once:
the stream subscribes before it reads the store, reads the revisions after
its cursor from the store (500 at a time) while it is behind, then tells
what its queue holds, skipping any revision it already told by number. A
cursor past the board's tip (a reader of another board, or of a board
restored) is answered with one `reset` to the tip, after which the reader
should read the board whole. `Last-Event-ID` or `after` not a number is
400 `last_event_id` or `query`.

**Slow readers.** Each stream has a bounded queue of 256 revisions. A
committed write publishes its new revisions to every queue without ever
waiting on a reader; a queue that would pass 256 is reset instead (its
revisions dropped), and that stream sends `reset` with its last revision
and ends. A reader that takes no bytes at all stalls the server's write to
it; the 10 s write deadline then drops the connection and the stream with
it, so a stalled reader holds at most its queue and its connection, never a
blocking thread or the writers. Either way the reader reconnects with
`Last-Event-ID` and is caught up from the store.

**Limits and the connection caps.**
- A stream is its connection's last reply. It ends at its connection's 60 s
  lifetime (not after the 15 s grace), and the reader reconnects from its
  last id: about one reconnect a minute per reader, so no stream holds a
  connection slot for good. The 10 s head timeout never ends a stream (no
  next request head is read while a reply is sent), and the 10 s write
  deadline counts only a write the reader is not taking, never an idle
  stream.
- At most 32 streams are open at once on a server, socket and TCP
  together, and 2 for one token; past either, 503 `busy`.
- Over TCP a stream holds one of its token's 8 connections for its life
  (#2746), so a token with 2 streams open has 6 left for its operations.
  A stream's request authenticates like any other, so it leaves its
  source's unauthenticated count at once.
- Each time a stream reads the store it holds one of its token's 4 blocking
  answers (out of 12 for TCP) for that read alone; idle, it holds no thread.

**`GET /v1/boards/{id}/revisions?after=<n>[&limit=<1..500>]`** answers the
revisions after `n`, oldest first, at most `limit` (500 when not given), as
revision events: `{"revisions": [...], "epoch": <current epoch>, "tip":
<newest revision>, "more": <whether revisions follow this batch>}`. The
query takes only `after` (required) and `limit`, each once, as a number
(else 400 `query`); a `limit` outside 1 to 500 is 400 `limit`.

Both routes are reads: over TCP any held token may ask them, on the Unix
socket the server's own user. An id that is not this server's board is 404
`board`.

**`quecto board watch [--url <url>] [--board-id <hex>] [--repo <path>]
[--remote <name>] [--after <revision>] [--count <n>]`** prints the feed,
one JSON line per event (the event's data as above), for scripts and the
Pocket relay's tests. The server, its token and the board's id are found
as for `quecto board --url`.
- It starts from `--after` (else now). When the stream ends or is cut, it
  opens it again from the last id it printed, after the stream's `retry:`
  (else 1 s), so each revision is printed once; after a `reset` it reads on
  from the reset's `after`.
- It prints each revision, each reset, and each change of epoch. An epoch
  the watch already printed, told again as a stream reopens, is not
  printed again, so an idle watch prints nothing as it reconnects.
  `--count <n>` counts these lines.
- It exits 0 after `--count` lines or once its output is closed. It exits
  1 when it cannot start, when the server refuses it, when a stream is
  malformed (its error said), or when 5 opens in a row fail or end with no
  event.
- A 503 `busy` is tried again, with a backoff doubling to 8 s, for up to
  75 s (past a connection's 60 s lifetime, by which a held stream gives
  its place back). Busy answers are counted apart from failed opens.
- A read waits at most 45 s (three missed 15 s pings) before the stream
  is opened again.

## Error replies

Every error reply the server makes is JSON, `{"error": "<why>", "code":
"<code>"}`:

| Status | `code` | When |
|---|---|---|
| 400 | `request` | an admin body that is not `{"name": "<name>"}`, or an op body not shaped as above |
| 400 | `query` | a feed query with a key other than `after`/`limit`, one twice, one not a number, or `revisions` without `after` |
| 400 | `limit` | a `revisions` limit outside 1 to 500 |
| 400 | `last_event_id` | a `Last-Event-ID` that is not a revision number |
| 400 | `write` | a write op without its `write_id` and `committer` |
| 400 | `token_name` | a token name that is not 1 to 64 of `a-z`, `0-9` and inner `-` |
| 401 | `token` | over TCP, no bearer token, or an unknown or revoked one (with `WWW-Authenticate: Bearer`) |
| 403 | `peer` | the peer is not the server's user |
| 403 | `local_only` | an admin route asked over TCP, or the op `bootstrap` or `clear_set_aside` over TCP |
| 404 | `board` | an op for a board id this server does not serve |
| 409 | `write_id_reused` | a `write_id` this principal used for another write |
| 404 | `no_token` | revoking a name no token has |
| 409 | `token_exists` | adding a name a token has |
| 409 | `token_limit` | adding a token past 256 |
| 404 | `route` | no such route |
| 404 | `no_board` | the repository has no board yet |
| 405 | `method` | a method the route does not take (`Allow` names those it does) |
| 408 | `timeout` | a body did not arrive within 10 s |
| 411 | `length` | a body without a `Content-Length` |
| 400 | `length` | a body not of its `Content-Length` |
| 400 | `body` | a body that could not be read |
| 413 | `too_large` | a body past 1 MiB |
| 500 | `handler` | the request's handler failed |
| 503 | `unavailable` | the board's store, the clock or the tokens could not answer (over TCP a token failure says only `the token store is unavailable`) |
| 503 | `clock_unsynced` | the host's clock is not synchronised, so a request that reads it is refused; `reason` says why |
| 503 | `busy` | over TCP, the caller already has 4 answers in flight, or all TCP callers together have 12; or 2 writes are already held; or the token already holds 8 TCP connections (this one is then closed); or a success's end while the remote is read for another write ("the remote is being read for another write: try again shortly", nothing written) |

Over TCP the token is checked first: an unauthenticated request is 401
before any limit or route answers it (only the HTTP parser's own bodyless
400 and 431, below, come first).

**Replies from the HTTP layer may have no body.** The HTTP parser refuses some requests before the server sees them, with an empty body and no code. Clients map these by status:

| Status | Client code | When |
|---|---|---|
| 400 | `bad_request` | a request that is not HTTP/1.1, or a `Content-Length` that is not a number (or two that differ) |
| 431 | `head_too_large` | more than 32 headers, or a head past 8 KiB |

## Limits

These are the limits in `infrastructure/http/board_service/limits.rs`. They
hold for the Unix socket and TCP alike:
- methods: `GET` and `POST`;
- bodies: only of a declared `Content-Length`, up to 1 MiB;
- 32 headers in 8 KiB;
- 10 s for a head, which also ends an idle keep-alive connection, and 10 s for a body;
- 10 s for a peer to take any reply byte;
- 64 connections at once on the socket, and 64 on each TCP address (TCP peers never take the socket's);
- a TCP connection must authenticate its first request within 2 s on loopback, 5 s from a Tailscale address, or it is closed (later requests on an authenticated connection have the usual 10 s);
- at most 8 TCP connections from one source that have not yet authenticated: a 9th evicts the source's oldest connection that has sent nothing (else its oldest), so idle sockets never lock out a client that authenticates at once. An authenticated connection leaves the count;
- at most 8 TCP connections at once for one token, across every TCP address: a connection takes one of its token's 8 when its token is first checked, and holds it until it closes. With none free it stays counted among its source's unauthenticated connections (above: evicted, and closed past its first-request time) while its body is read, then it is answered 503 `busy` whole and closed. A body still arriving at the first-request deadline is dropped unanswered, like any unauthenticated connection; so 408 comes only on a connection already placed that switches to a full token. A request its head already refuses (405, 404, 411, 413), or whose body fails (400, 408), is answered as itself and closed. So one token holds at most 8 authenticated connections, and its refused ones count among each source's 8 unauthenticated ones: no more than a sender with no token can already hold. The Unix socket has no such cap;
- the token is checked without a blocking thread. Answers that need one (a store or clock read) are capped over TCP: 4 at once per token, 12 for all TCP callers together, out of the server's 16. So a slow remote can never take the threads the Unix socket needs (`token list`, `token revoke`);

**The loopback trade-off.** All of loopback (`127.0.0.0/8` and `::1`) counts as one source, since any local user can bind any of it. So any user on the host can open 8 silent connections and evict a loopback client that is slow to send its first request. A client that sends its request at once is never evicted, and a Tailscale device has its own source. Listen on loopback only on a host whose users you trust, or have clients send their request as soon as they connect;
- each connection at most 60 s or 1,000 requests, then 15 s to finish what it is answering before it is dropped.


## Offline native history export (B3-A)

`quecto board-server export --repo <destination> --board-id <16-hex>
--to-branch <branch> --format 3 [--state <dir>] [--dry-run]` reads the
existing native schema-4 SQLite board that `serve` keeps:
`board.sqlite3` in the server's state directory (`--state <dir>`, else
`<base>/board-server/<board-id>/`, where `<base>` is `QUECTO_BASE_DIR`,
else `~/.quecto`). The directory must be absolute and not end in `/`. A
missing board file is refused by name; export never makes one.
The board ID must be exactly 16 hexadecimal characters; it is not a path.
Only format `3` is accepted. The optional `--remote <name>` is validated
for command compatibility; native export does not contact or fetch a remote.
Unknown or duplicate flags, unsupported formats, and invalid target refs
are refused. `board-server export --help` describes this command.

This offline command runs before token or server dispatch: it starts no
listener, opens no writable board connection, and creates no source DB or
data directory. Earlier schemas (2, and 3 before B7) and foreign schemas
are refused without migration or reset. Imported originals require the
destination to use their exact SHA1 or SHA256 format; validated pure-native
history permits either.

A readonly transaction pins the complete native revision frontier. Export
writes deterministic real Git objects in the destination's SHA1 or SHA256
format, retaining each successful revision's writer, message, UTC timestamp
and actual preceding Git parent, including unchanged writes and clears.
There is no total native revision cap; traversal uses a 4096-pair ID-only
cache, checked object/listing bounds and a depth limit of 128. These are
finite algorithmic bounds, not an RSS guarantee.

The result on stdout is JSON with `revisions`, `frontier` (epoch:revision),
and `git_head`. Dry-run performs the full verification and hashing but writes
neither objects nor refs. A real export verifies the entire captured history
before publishing the target using the expected head captured by composition.
A competing head is a conflict; an operational lock failure with the same
expected head is unavailable. An ambiguous publication is not success: reconcile
the expected and proposed Git heads against the destination and the captured
source frontier before retrying. Failed publication may leave verified
unreachable destination objects. No distributed DB/Git atomic transaction is
claimed, and export never mutates source board rows. Existing `serve`, `token`,
HTTP routes, authentication, and timing behavior are unchanged.


## Offline complete history import (B3-B)

`quecto board-server import --repo <source> --board-id <16-hex>
[--remote <name>] [--state <dir>]` captures the complete local
`refs/heads/quecto/board` H0 once, into the board file `serve` keeps:
`board.sqlite3` in the server's state directory (`--state <dir>`, else
`<base>/board-server/<board-id>/`, where `<base>` is `QUECTO_BASE_DIR`,
else `~/.quecto`). The directory passes the same checks as `serve`'s
(absolute, no trailing `/`, no symlink, this user's at mode 0700, made so
if missing), and import holds `serve`'s lock on it until it is done, so an
import while a server runs on that directory is refused. Give the board id
the server announces (`serving board <id>`) and the server's `--state`, if
any. A board file is bound to one board id: an import records its
`--board-id`; a server binds an unbound board file to its own board id at
start, under its directory lock, and a board file it first makes is bound
by its first write. A server whose board id differs from the file's
refuses to start, naming both ids and the file, and `export` with another
`--board-id` is refused. `--remote` defaults to `origin` and is provenance only: no fetch,
listener or token graph is started. `import --help` describes the command;
stdout JSON contains `revisions`, `head` and `source_head`.

An owned IMMEDIATE transaction requires every table of the schema empty.
It preserves original raw commits/trees/blobs, IDs, headers and exposed
clear/seen/keep tokens; the SAME complete export dry-run verifies every
original and H0 before commit. Error, Drop and owned precommit crash roll
back every row/provenance; a newly initialized empty schema file may remain.
Nonempty targets conflict without overwrite, staging or resume. Original
replay through H0 and all later native children use the same exporter.

Only complete local linear SHA1/SHA256 histories are supported; shallow,
promisor, nonlinear, missing or malformed histories are refused wholly.
Inclusive bounds: 4096 commits, 32768 unique blob/tree objects, 128 MiB raw
including commits, 64 MiB blob, 64 KiB commit, 655360-byte tree, 65536 projected
entries, 32 MiB metadata, 4096-byte raw names and tree depth128. Runner and
Reader copies are additional bounded allocations, not an RSS guarantee.
Scratch owner12/corpus106 in both formats prove the offline path, not a live
cut-over. No live import, fence, deployment or service start is performed.
