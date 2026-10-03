# quecto-mcp

`quecto-mcp` is a standalone Quecto UDS extension binary that exposes remote MCP server tools as Quecto tools.

It connects to:

- a running Quecto agent Unix socket, and
- an MCP server such as Perme8 `perme8-mcp`.

It discovers MCP tools, filters them, maps MCP names to Quecto-safe names, registers them with Quecto via `register_tools`, and proxies Quecto `execute_tool` events to MCP `tools/call`.

## Example

```bash
quecto agent --mode uds --socket /tmp/quecto-agent-community-1.sock --persist

quecto-mcp \
  --socket /tmp/quecto-agent-community-1.sock \
  --mcp-url https://perme8.example.com/mcp \
  --mcp-token "$PERME8_MCP_TOKEN" \
  --tool-prefix community. \
  --register-timeout 10
```

Equivalent environment variables:

```bash
QUECTO_SOCKET=/tmp/quecto-agent-community-1.sock \
PERME8_MCP_URL=https://perme8.example.com/mcp \
PERME8_MCP_TOKEN=... \
QUECTO_MCP_TOOL_PREFIXES=community. \
quecto-mcp
```

## Options

Required:

- `--socket` or `QUECTO_SOCKET`
- `--mcp-url` or `PERME8_MCP_URL`
- `--mcp-token`, `--mcp-token-file`, `--mcp-token-command`, or `PERME8_MCP_TOKEN`. Explicit file and command sources are fatal if they cannot produce a non-empty token; command sources must exit successfully within 10 seconds. Later CLI token source options override earlier token sources.

Optional:

- `--mcp-server-name`, default `perme8-mcp`
- `--tool-prefix`, repeatable; defaults to `community.` when no allowlist is configured
- `--tool-allowlist`, comma-separated MCP tool names. When present without `--tool-prefix`, it disables the default `community.` prefix filter.
- `--tool-denylist`, comma-separated MCP tool names; denylist wins over allowlist and prefix matches.
- `--name-prefix`, prefix for registered Quecto tool names; the final registered names must still be Quecto-safe
- `--timeout`, MCP HTTP timeout in seconds (default 30, or `QUECTO_MCP_TIMEOUT_SECONDS`). Each tool registers with Quecto with `timeoutSeconds` of this plus 5 seconds, within Quecto's 1 to 600, so Quecto waits as long as an MCP call may take. A call that has to re-initialize the MCP session and retry can take longer than that.
- `--register-timeout`, Quecto `register_tools` timeout in seconds
- `--refresh-interval` is reserved for deployment compatibility but is not implemented. `quecto-mcp` rejects this option; restart `quecto-mcp` to refresh tool registrations.

## Tool name mapping

MCP names are mapped to Quecto-safe names by replacing non-alphanumeric characters with underscores:

```text
community.feed.list             -> community_feed_list
community.channels.send_message -> community_channels_send_message
community.chat.send_dm          -> community_chat_send_dm
```

Collisions, duplicate MCP names, invalid MCP names, and invalid final names after `--name-prefix` fail closed.

## Protocol notes

`quecto-mcp` uses JSON-RPC over HTTP for MCP requests. It sends an `Accept: application/json, text/event-stream` compatibility header, but this bridge currently expects JSON response bodies and does not implement SSE stream parsing.

Quecto UDS messages are newline-delimited JSON. For `execute_tool`, `arguments` is expected to be a JSON string containing the MCP argument object. Malformed JSON or non-string `arguments` values produce a deterministic `tool_result` with `isError: true` when the tool call id and tool name can be read.

## Tool results

An MCP `tools/call` result becomes one Quecto `tool_result`:

- `text` items are joined with newlines into `content`.
- `image` items (`{"type":"image","data","mimeType"}`) become `imageBlocks`, admitted by the `quecto-image` crate with the rules the agent applies: PNG, JPEG, GIF or WebP; strict standard base64; at most 3.75 MiB decoded; the type's signature; a readable header. At most 8 are sent.
- An image that cannot be sent is a marker in `content` in its place, `[image omitted: <reason>]`: the refusal text (e.g. `[image omitted: mimeType "image/svg+xml" is not allowed; use image/png, image/jpeg, image/gif or image/webp]`), a malformed item, the ninth image onwards, or an image the 8 MiB UDS line has no room for.
- A result with `isError: true` (an MCP tool error) is an error `tool_result`, with its text and images as for any result.
- A result with neither text nor images (only `resource` items, say) is sent as its JSON, as before. MCP responses are read up to 16 MiB; a result whose text alone cannot fit the UDS line is an error result saying so.

## Security model

Run one `quecto-mcp` process per Quecto community agent. The MCP token should be agent-scoped and least-privilege. `quecto-mcp` never accepts actor identity from model-controlled tool arguments.
