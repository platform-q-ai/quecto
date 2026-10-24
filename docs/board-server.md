# Board server (`quecto board-server`)

`quecto board-server serve --socket <path> --repo <clone> [--remote origin]
[--listen <ip:port>]... [--state <dir>]` serves a repository's project
board over HTTP/1.1 on a Unix socket, and on each allowlisted TCP address,
until SIGINT or SIGTERM (epic #2705 B4, B5).

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
  detail is said on the Unix socket, and goes to the server's stderr at
  most once every 30 s, with a count of the lines held back.
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
`~/.quecto/board-server/<board id>/`. A token is compared in constant
time. A server holds at most 256 tokens. The server holds an exclusive
lock on the state directory itself for its whole life, so a second
server on the same state directory (say, a second clone of the same
remote) is refused at start; give it its own `--state <dir>`. If the
directory is moved away and replaced while a server runs, that server
refuses every token read and write. (After B2 the tokens move into the
SQLite store as admin revisions.)

## Routes

| Route | Answer |
|---|---|
| `GET /v1/status` | `{"role": "primary", "board": "<id>", "head": "<head>" or null}` |
| `GET /v1/time` | `{"time": "<RFC 3339, to the second>"}` (the server's clock) |
| `GET /v1/admin/tokens` | `{"tokens": [{"name": "<name>", "created": "<RFC 3339>"}]}` (Unix socket only) |
| `POST /v1/admin/tokens` `{"name": "<name>"}` | 201 `{"name", "created", "token"}`: the one time the token is shown (Unix socket only) |
| `POST /v1/admin/tokens/revoke` `{"name": "<name>"}` | `{"name": "<name>", "revoked": true}` (Unix socket only) |

The board snapshot route comes with B11. Writes (`/op`) come with B7.

## Error replies

Every error reply the server makes is JSON, `{"error": "<why>", "code":
"<code>"}`:

| Status | `code` | When |
|---|---|---|
| 400 | `request` | an admin body that is not `{"name": "<name>"}` |
| 400 | `token_name` | a token name that is not 1 to 64 of `a-z`, `0-9` and inner `-` |
| 401 | `token` | over TCP, no bearer token, or an unknown or revoked one (with `WWW-Authenticate: Bearer`) |
| 403 | `peer` | the peer is not the server's user |
| 403 | `local_only` | an admin route asked over TCP |
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
- at most 8 TCP connections at once from one source address; a 9th is closed unanswered. All of loopback (`127.0.0.0/8` and `::1`) counts as one source, since any local user can bind any of it;
- each connection at most 60 s or 1,000 requests, then 15 s to finish what it is answering before it is dropped.
