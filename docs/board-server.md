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
  unknown or revoked token, the answer is 401 (before any route is looked
  up). The admin routes answer 403 over TCP, whatever the token.

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
time. A server holds at most 256 tokens. (After B2 the tokens move into
the SQLite store as admin revisions.)

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
| 503 | `unavailable` | the board's store, the clock or the tokens could not answer |

The limits answer before a token is checked: over TCP, a request refused
by a limit (405, 408, 411, 413, or one of the bodyless replies below) is
refused whatever its token.

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
- 64 connections at once, across the socket and every TCP address;
- each connection at most 60 s or 1,000 requests, then 15 s to finish what it is answering before it is dropped.
