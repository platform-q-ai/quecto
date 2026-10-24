# Board server (`quecto board-server`)

`quecto board-server serve --socket <path> --repo <clone> [--remote origin]`
serves a repository's project board over HTTP/1.1 on a Unix socket
until SIGINT or SIGTERM (epic #2705 B4).

Only a process of the server's own user may connect, checked by peer
credentials. The socket is mode 0600 in a directory of mode 0700.

## Routes

| Route | Answer |
|---|---|
| `GET /v1/status` | `{"role": "primary", "board": "<id>", "head": "<head>" or null}` |
| `GET /v1/time` | `{"time": "<RFC 3339, to the second>"}` (the server's clock) |

The board snapshot route comes with B11. Writes (`/op`) come with B7.

## Error replies

Every error reply the server makes is JSON, `{"error": "<why>", "code":
"<code>"}`:

| Status | `code` | When |
|---|---|---|
| 403 | `peer` | the peer is not the server's user |
| 404 | `route` | no such route |
| 404 | `no_board` | the repository has no board yet |
| 405 | `method` | a method the route does not take (`Allow` names those it does) |
| 408 | `timeout` | a body did not arrive within 10 s |
| 411 | `length` | a body without a `Content-Length` |
| 400 | `length` | a body not of its `Content-Length` |
| 400 | `body` | a body that could not be read |
| 413 | `too_large` | a body past 1 MiB |
| 500 | `handler` | the request's handler failed |
| 503 | `unavailable` | the board's store or clock could not answer |

**Replies from the HTTP layer may have no body.** The HTTP parser refuses some requests before the server sees them, with an empty body and no code. Clients map these by status:

| Status | Client code | When |
|---|---|---|
| 400 | `bad_request` | a request that is not HTTP/1.1, or a `Content-Length` that is not a number (or two that differ) |
| 431 | `head_too_large` | more than 32 headers, or a head past 8 KiB |

## Limits

These are the limits in `infrastructure/http/board_service/limits.rs`:
- methods: `GET` and `POST`;
- bodies: only of a declared `Content-Length`, up to 1 MiB;
- 32 headers in 8 KiB;
- 10 s for a head, which also ends an idle keep-alive connection, and 10 s for a body;
- 10 s for a peer to take any reply byte;
- 64 connections at once;
- each connection at most 60 s or 1,000 requests, then 15 s to finish what it is answering before it is dropped.
