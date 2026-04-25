# PRD: `quecto-mcp` — MCP Client UDS Extension for Quecto

## Overview

Build `quecto-mcp`, a Quecto companion process that connects to a running Quecto agent over the existing Unix Domain Socket (UDS) extension bus and exposes remote MCP server tools as Quecto tools.

The immediate target integration is Perme8’s `perme8-mcp` server, specifically its user-level `community.*` tools. `quecto-mcp` should allow a Quecto-backed community agent to discover and call Perme8 MCP tools while preserving Quecto’s existing architecture: the LLM only sees Quecto tools, and external tool providers register through the UDS extension protocol.

This should work similarly to how `quecto-tui` connects to a Quecto UDS agent, but instead of acting as a UI client, `quecto-mcp` acts as a UDS extension client that registers tools and services `execute_tool` requests.

## Problem Statement

Quecto already supports external tools through its UDS extension bus. Perme8 now exposes community capabilities through `perme8-mcp`, but Quecto cannot use those tools directly unless they are registered into Quecto’s tool registry.

Passing MCP configuration through `quecto-api`’s `/prompt` request is not a good fit because:

- `quecto-api` currently accepts only a message and optional streaming behavior.
- Quecto’s UDS `prompt` command does not include runtime MCP metadata.
- Tool registration belongs in Quecto’s UDS extension system, not in prompt payloads.
- Prompt metadata would not by itself register tools in the LLM’s tool list.

The correct integration is a UDS extension process that bridges Quecto tool calls to MCP `tools/call` requests.

## Goals

- Provide a `quecto-mcp` binary or subcommand that connects to a Quecto UDS socket.
- Connect to one or more MCP servers, initially targeting `perme8-mcp` over HTTP/StreamableHTTP.
- Discover MCP tools from the remote server.
- Filter tools by configured allowlist/prefixes, initially `community.*`.
- Register filtered MCP tools into Quecto using the existing `register_tools` UDS command.
- Translate Quecto `execute_tool` events into MCP `tools/call` requests.
- Return MCP results to Quecto using `tool_result`.
- Preserve one Quecto instance to one community agent identity.
- Use one agent-scoped MCP credential per `quecto-mcp` process.
- Avoid introducing implicit user impersonation or runtime actor switching.

## Non-Goals

- Do not modify the LLM provider tool-calling loop unless required for name/schema compatibility.
- Do not pass MCP metadata through `/prompt` as the primary integration mechanism.
- Do not make one Quecto process serve many Perme8 community agent identities.
- Do not let the model choose the security actor via tool arguments.
- Do not expose all MCP tools by default.
- Do not require Perme8-specific logic in Quecto core beyond a generic MCP client bridge.
- Do not replace the existing UDS extension mechanism.

## Desired Runtime Topology

For each Perme8 community agent:

```text
quecto agent --mode uds --socket /tmp/quecto-agent-<agent-id>.sock --persist

quecto-api
  -> connects to /tmp/quecto-agent-<agent-id>.sock
  -> exposes HTTP /prompt for Perme8 QuectoGateway

quecto-mcp
  -> connects to /tmp/quecto-agent-<agent-id>.sock
  -> connects to Perme8 perme8-mcp
  -> registers allowed MCP tools as Quecto UDS extension tools
```

Community DM flow:

```text
Perme8 user DM to community agent
  -> JargaCommunityWeb.Agents.QuectoGateway
  -> quecto-api /prompt for that agent
  -> quecto agent LLM
  -> LLM calls registered tool, e.g. community_channels_send_message
  -> quecto agent emits execute_tool to quecto-mcp
  -> quecto-mcp calls perme8-mcp community.channels.send_message
  -> quecto-mcp returns tool_result
  -> Quecto continues/replies
```

## Key Principle: One Quecto Instance = One Agent Identity

Each Quecto instance should have a 1:1 relationship with one Perme8 community agent.

`quecto-mcp` should therefore be configured with one MCP credential representing that specific agent. Every MCP call made by that `quecto-mcp` process executes as that agent.

This avoids unsafe identity switching and avoids relying on the LLM to provide trusted identity arguments.

## Users

### Quecto Operator

Runs a Quecto agent, `quecto-api`, and `quecto-mcp` for a specific community agent.

### Perme8 Community Agent

The agent identity whose MCP credential is used by `quecto-mcp`.

### Quecto LLM Agent

Sees MCP-backed tools as normal Quecto tools and may invoke them during reasoning.

### Perme8 MCP Server

Receives authenticated MCP tool calls from `quecto-mcp`.

## CLI Requirements

`quecto-mcp` should support a CLI like:

```bash
quecto-mcp \
  --socket /tmp/quecto-agent.sock \
  --mcp-url https://perme8.example.com/mcp \
  --mcp-token "$PERME8_AGENT_MCP_TOKEN" \
  --tool-prefix community.
```

Equivalent subcommand form is acceptable:

```bash
quecto mcp \
  --socket /tmp/quecto-agent.sock \
  --mcp-url https://perme8.example.com/mcp \
  --mcp-token "$PERME8_AGENT_MCP_TOKEN" \
  --tool-prefix community.
```

Required options:

- `--socket`: Quecto UDS socket path.
- `--mcp-url`: MCP server URL.
- `--mcp-token` or equivalent credential source.

Optional options:

- `--mcp-server-name`, default `perme8-mcp`.
- `--tool-prefix`, repeatable. Example: `community.`.
- `--tool-allowlist`, comma-separated MCP tool names.
- `--tool-denylist`, comma-separated MCP tool names.
- `--name-prefix`, optional prefix for registered Quecto tool names.
- `--refresh-interval`, optional interval to re-list MCP tools and re-register changed definitions.
- `--timeout`, MCP call timeout.
- `--register-timeout`, Quecto registration timeout.
- `--stdio-log` or normal tracing verbosity flags.

Environment variables should be supported:

```text
QUECTO_SOCKET=/tmp/quecto-agent.sock
PERME8_MCP_URL=https://perme8.example.com/mcp
PERME8_MCP_TOKEN=...
QUECTO_MCP_TOOL_PREFIXES=community.
QUECTO_MCP_TIMEOUT_SECONDS=30
```

## Tool Discovery Requirements

On startup, `quecto-mcp` must connect to the MCP server and discover available tools.

For MCP servers, this means performing the MCP initialize/session handshake and calling tool listing according to the server’s transport/protocol.

For each discovered tool, `quecto-mcp` should collect:

- MCP tool name
- description
- input schema

Then it should apply configured filtering:

- include tools matching any `--tool-prefix`, if provided
- include only `--tool-allowlist`, if provided
- exclude `--tool-denylist`, if provided

Default behavior should be conservative. For the Perme8 Community use case, the default deployment should include only `community.*` tools.

## Quecto Tool Name Mapping

MCP tool names may contain characters that are awkward or invalid for LLM function/tool names, such as dots.

`quecto-mcp` should register Quecto-safe names while keeping a reversible mapping to MCP tool names.

Example mapping:

```text
community.feed.list              -> community_feed_list
community.channels.send_message  -> community_channels_send_message
community.chat.send_dm           -> community_chat_send_dm
```

Requirements:

- Name mapping must be deterministic.
- Name mapping must avoid collisions.
- If two MCP names map to the same Quecto name, startup/registration must fail or disambiguate predictably.
- Error messages must identify both the Quecto-safe name and original MCP name.

## Tool Registration Requirements

After discovery/filtering/mapping, `quecto-mcp` must connect to the Quecto UDS socket and send `register_tools`.

Example:

```json
{
  "type": "register_tools",
  "id": "register-mcp-tools-1",
  "tools": [
    {
      "name": "community_feed_list",
      "description": "List community feed posts visible to the agent",
      "parametersSchema": "{\"type\":\"object\",\"properties\":{...}}"
    }
  ]
}
```

Registration behavior:

- If registration succeeds, `quecto-mcp` begins listening for `execute_tool` events.
- If registration fails because a tool shadows a Quecto core tool, `quecto-mcp` should log the failure and exit non-zero.
- If registration fails for a subset of tools, behavior should be configurable, but fail-fast is preferred initially.
- If disconnected from Quecto, tools naturally unregister through Quecto’s existing UDS extension lifecycle.

## Tool Execution Requirements

When Quecto emits:

```json
{
  "type": "execute_tool",
  "toolCallId": "uds-...",
  "toolName": "community_feed_list",
  "arguments": "{\"limit\":10}"
}
```

`quecto-mcp` must:

1. Look up `community_feed_list` in its mapping table.
2. Resolve original MCP tool name, e.g. `community.feed.list`.
3. Parse `arguments` as JSON.
4. Call MCP `tools/call` with the original MCP name and JSON arguments.
5. Convert the MCP response into text content suitable for Quecto `tool_result`.
6. Send:

```json
{
  "type": "tool_result",
  "toolCallId": "uds-...",
  "content": "...",
  "isError": false
}
```

If MCP returns an error, `quecto-mcp` must return:

```json
{
  "type": "tool_result",
  "toolCallId": "uds-...",
  "content": "...error summary...",
  "isError": true
}
```

## MCP Transport Requirements

Initial required transport:

- HTTP/StreamableHTTP compatible with Perme8 `perme8-mcp`.

The implementation should support:

- bearer authorization header
- session initialization
- session ID preservation when required by the MCP server
- `tools/list`
- `tools/call`
- JSON response/error decoding
- reconnect/reinitialize after recoverable transport/session errors

If using an MCP client crate, it must support the protocol version used by Perme8’s Hermes MCP server.

## Credential Requirements

`quecto-mcp` must authenticate to MCP using an agent-scoped credential.

Requirements:

- Credential represents the one Perme8 community agent associated with this Quecto instance.
- Credential is not a human sender credential.
- Credential is not a shared global all-powerful key.
- Credential has least-privilege MCP scopes.
- Credential is never logged.
- Authorization headers are redacted in logs/errors.

Preferred credential source:

```text
PERME8_MCP_TOKEN
```

Alternative sources may include:

- file path to a token
- command that returns a short-lived token
- future token broker integration

## Scope Requirements

Scope enforcement primarily happens on the MCP server. However, `quecto-mcp` should avoid registering tools the agent is not intended to use.

Suggested deployment defaults for community agents:

Read-oriented tools:

```text
community.feed.list
community.feed.get
community.channels.list
community.channels.get
community.channels.list_messages
community.chat.list_conversations
community.chat.list_dms
community.members.list
community.members.get
```

Optional write tools:

```text
community.feed.create
community.feed.reply
community.feed.react
community.feed.bookmark
community.channels.send_message
community.channels.react
community.chat.send_dm
```

High-impact tools should require explicit allowlisting:

```text
community.channels.create
community.channels.update
community.channels.archive
community.channels.invite
community.channels.remove_member
community.channels.delete_message
community.chat.delete_dm
community.voice.token
```

## Lifecycle Requirements

`quecto-mcp` should be a long-running companion process.

Startup sequence:

1. Load configuration.
2. Connect to MCP server.
3. Initialize MCP session.
4. List tools.
5. Filter/map tools.
6. Connect to Quecto UDS socket.
7. Register tools.
8. Listen for `execute_tool` events.

Runtime behavior:

- Continue serving tool calls until interrupted or disconnected.
- If MCP connection fails during a tool call, return tool error to Quecto.
- If Quecto UDS disconnects, exit or retry depending on configuration.
- On SIGINT/SIGTERM, unregister tools when possible or close UDS cleanly.

Reconnect behavior:

- Initial implementation may exit on Quecto disconnect and rely on supervisor restart.
- Initial implementation may reconnect/reinitialize MCP after transient MCP session errors.
- Future implementation may support full reconnect loops for both Quecto and MCP.

## Observability Requirements

`quecto-mcp` should log:

- startup configuration summary with secrets redacted
- MCP server URL host/path, not token
- number of MCP tools discovered
- number of tools registered into Quecto
- each tool execution start/end with tool name and duration
- MCP errors with secrets redacted
- Quecto UDS disconnect/reconnect events

`quecto-mcp` must not log:

- bearer tokens
- authorization headers
- raw credentials
- private MCP response bodies at debug/info if they may contain private community data

## Security Requirements

- Use least-privilege agent-scoped MCP credentials.
- Never use sender credentials.
- Never let the model decide actor identity.
- Do not expose tools outside configured allowlist/prefixes.
- Redact secrets from logs.
- Respect Quecto’s UDS socket permissions and extension lifecycle.
- Return MCP permission failures to the model as tool errors rather than retrying with broader credentials.
- Fail closed when tool-name mapping is ambiguous.

## Backward Compatibility

This should not break existing Quecto behavior.

- Quecto can still run without `quecto-mcp`.
- `quecto-api /prompt` remains unchanged.
- Existing UDS extension protocol remains unchanged if possible.
- Existing `quecto-tui` behavior remains unchanged.
- Existing native and UDS extension tools remain supported.

## Acceptance Criteria

### Basic Startup

- Given a running Quecto UDS agent and valid MCP server/token, `quecto-mcp` starts successfully.
- `quecto-mcp` discovers MCP tools.
- `quecto-mcp` registers filtered tools into Quecto.
- `get_extensions` or equivalent Quecto state reflects the registered tools.

### Tool Name Mapping

- `community.feed.list` is registered as `community_feed_list`.
- `community.channels.send_message` is registered as `community_channels_send_message`.
- Mapping from Quecto-safe name back to MCP name is correct during execution.
- Mapping collisions are detected and handled safely.

### Tool Execution

- When Quecto emits `execute_tool` for a registered MCP-backed tool, `quecto-mcp` calls the corresponding MCP tool.
- Successful MCP results are returned as Quecto `tool_result` with `isError: false`.
- MCP errors are returned as Quecto `tool_result` with `isError: true`.
- Invalid JSON arguments produce a Quecto tool error.
- Unknown Quecto tool names produce a Quecto tool error.

### Perme8 Community Use Case

- A Quecto instance configured for one Perme8 community agent can see `community.*` tools.
- The Quecto agent can call an allowed community MCP tool.
- MCP calls execute as the configured agent credential.
- MCP denies calls outside the configured token scopes.
- `quecto-mcp` does not use sender identity or prompt text to decide credentials.

### Security

- Tokens are not printed in logs.
- Authorization headers are redacted from error messages.
- Tools outside the configured prefix/allowlist are not registered.
- A global/unscoped token is not required.

### Resilience

- MCP transport errors produce tool errors rather than crashing the Quecto agent.
- Quecto UDS disconnect causes `quecto-mcp` to exit or retry according to configuration.
- `quecto-mcp` shuts down cleanly on SIGINT/SIGTERM.

## Suggested Delivery Phases

### Phase 1: Minimal UDS Extension Bridge

- Add `quecto-mcp` binary or `quecto mcp` subcommand.
- Connect to Quecto UDS.
- Register a configured static list of tools.
- Handle `execute_tool` and return canned/test results.
- Add unit tests for UDS registration and execution protocol handling.

### Phase 2: MCP Client Integration

- Implement MCP client initialization.
- Implement `tools/list`.
- Implement `tools/call`.
- Register discovered tools from MCP.
- Add tests using a fake MCP server.

### Phase 3: Perme8 Community Defaults

- Add default tool-prefix/allowlist support for `community.*`.
- Add Quecto-safe name mapping.
- Add tests for Perme8-style dotted tool names.
- Document deployment alongside `quecto-api` and Perme8 QuectoGateway.

### Phase 4: Production Hardening

- Add reconnect/reinitialize behavior.
- Add token-from-command or token-file support.
- Add metrics/tracing.
- Add supervisor/systemd/container examples.
- Add end-to-end test with Quecto UDS agent and fake MCP server.

## Open Questions

- Should `quecto-mcp` live as a separate binary crate, e.g. `quecto-mcp`, or as a `quecto mcp` subcommand?
- Which Rust MCP client crate, if any, should be used?
- Does Perme8’s Hermes StreamableHTTP server require any transport-specific handling beyond normal MCP initialization and session headers?
- Should tool listing be refreshed periodically, or only at startup?
- Should `quecto-mcp` register all allowed tools as flat Quecto tools, or also add a single generic `mcp_call` escape hatch?
- Should there be a built-in Perme8 profile that defaults to `PERME8_MCP_URL`, `PERME8_MCP_TOKEN`, and `community.*`?
- Should `quecto-mcp` support multiple MCP servers in one process, or should the 1:1 identity model prefer one server profile per process?
