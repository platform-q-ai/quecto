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
- `--mcp-token`, `--mcp-token-file`, `--mcp-token-command`, or `PERME8_MCP_TOKEN`

Optional:

- `--mcp-server-name`, default `perme8-mcp`
- `--tool-prefix`, repeatable; defaults to `community.` when no allowlist is configured
- `--tool-allowlist`, comma-separated MCP tool names
- `--tool-denylist`, comma-separated MCP tool names
- `--name-prefix`, prefix for registered Quecto tool names
- `--timeout`, MCP HTTP timeout in seconds
- `--register-timeout`, Quecto `register_tools` timeout in seconds
- `--refresh-interval`, parsed for deployment compatibility; currently tool registration happens at startup

## Tool name mapping

MCP names are mapped to Quecto-safe names by replacing non-alphanumeric characters with underscores:

```text
community.feed.list             -> community_feed_list
community.channels.send_message -> community_channels_send_message
community.chat.send_dm          -> community_chat_send_dm
```

Collisions fail closed.

## Security model

Run one `quecto-mcp` process per Quecto community agent. The MCP token should be agent-scoped and least-privilege. `quecto-mcp` never accepts actor identity from model-controlled tool arguments.
