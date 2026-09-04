# Extending Quecto

Use this page to route user requests about adding tools, models, providers, or clients. Do not edit harness source unless the user is changing Quecto itself.

## Add tools

For new agent-callable tools:

- Prefer UDS extensions: an external process connects to the agent socket and registers tools.
- MCP tools usually come through the MCP bridge, then appear as normal Quecto tools.
- Native extensions are for tools compiled into Quecto and enabled by config.

See `docs {"name":"extensions"}` for extension mechanics.

## Add models or providers

For new model metadata or API-key providers, edit:

```text
~/.quecto/models.json
```

Do not edit source code for normal model/provider additions.

Valid changes hot-reload before prompts, model switches, `/model` opens, and explicit reloads. Use env-var credential references such as `$OPENROUTER_API_KEY`; do not write literal secrets into project files.

See `docs {"name":"models"}` for schema and procedure.

## Add clients

Clients should connect to a running `quecto agent --mode uds` process rather than embedding or modifying the harness.

Use the UDS protocol for commands/events. Existing examples include the TUI, HTTP/WebSocket gateway, MCP bridge, and runtime manager.
