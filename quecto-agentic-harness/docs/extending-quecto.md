# Extending Quecto

Quecto can be extended without modifying the harness source. The core agent exposes a UDS event bus for clients and tool providers, and it loads model/provider metadata from user configuration.

## Add tools

Use extensions when you want Quecto agents to call tools beyond the built-in set.

- **UDS extensions** are external processes that connect to a running agent socket, register tool definitions, execute tool calls, and return results. They can be written in any language that can speak the length-prefixed JSON UDS protocol.
- **MCP tools** can be exposed through `quecto-mcp`, which bridges remote MCP servers into Quecto tools over the same UDS extension mechanism.
- **Native extensions** are Rust tools compiled into Quecto and enabled by config. They are mainly for tools that ship with the harness.

Start with the [Extensions guide](extensions.md). For the raw integration contract, see the `register_tools`, `execute_tool`, and `tool_result` commands in the [UDS protocol reference](uds-protocol.md#register_tools).

## Add models and providers

Use the runtime model registry when you want Quecto to use another model or OpenAI-compatible provider without rebuilding the binary.

The registry lives at:

```text
~/.quecto/models.json
```

For OpenAI-compatible providers, add a provider entry with its `baseUrl`, auth mode, and model list. Quecto hot-reloads the registry before prompts, model switches, `/model` opens, and explicit reloads, so saving the file is enough for a running agent to pick up the change.

For providers that expose a `/models` endpoint, `quecto models discover <provider>` can refresh that provider's model list while preserving the rest of the registry.

See [Runtime models and providers](runtime-models-providers.md) for the schema, auth modes, discovery command, and examples.

## Add clients

Clients do not embed Quecto. They connect to a running `quecto agent --mode uds` process and send commands over the UDS protocol while receiving streamed events. This is how the terminal UI, HTTP/WebSocket API gateway, MCP bridge, and runtime manager stay separate from the core harness.

See the [UDS protocol reference](uds-protocol.md) for command/event details and [quecto-line-io](../../quecto-line-io/README.md) for the shared bounded framing library.
