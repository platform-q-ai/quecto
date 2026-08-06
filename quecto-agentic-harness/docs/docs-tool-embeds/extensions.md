# Extensions (deep dive)

Tool names and schemas for enabled extensions already appear in your tool list. This page is how extensions are added.

## Two mechanisms

| Kind | How | Runtime |
|---|---|---|
| **Native** | `config.json` `tools.*` (e.g. web search/fetch) | Process start; children re-read the same config |
| **UDS** | External client: `register_tools` on the agent socket | Connect = available; disconnect = auto-unregister |

## Rules that matter mid-task

- Cannot shadow core tool names (`bash`, `read`, `write`, `edit`, `ls`, `grep`, `find`, `rust_ast_graph`, `docs`, `recall`, `spawn`, `agent_cmd`, `workflow`, …).
- `--disable-tool` keeps descriptors registered, hides tools from the model, rejects execution, and deny-lists names for the process; UDS cannot re-add them.
- `get_tool_catalogue` / `list_tools` returns the rich bundled-native+UDS `ToolCatalogueEntry` snapshot for control/query clients; runtime providers still use `register_tools` / `unregister_tools` / `execute_tool` / `tool_result`.
- UDS tool default timeout ~30s; disconnect mid-call → error result.

## Web (if enabled)

- Prefer Brave when `tools.web.brave` + API key (`QUECTO_TOOLS_WEB_BRAVE_API_KEY` or config); else DuckDuckGo.
- `web_fetch` is for full page text after search — respect tool schema limits.

## See also

- Entry: `docs {"name":"quick-start"}`
- Full human reference: `docs/extensions.md` in the repo
