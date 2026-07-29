# Extensions (deep dive)

Tool names and schemas for enabled extensions already appear in your tool list. This page is how extensions are added.

## Two mechanisms

| Kind | How | Runtime |
|---|---|---|
| **Native** | `config.json` `tools.*` (e.g. web search/fetch) | Process start; children re-read the same config |
| **UDS** | External client: `register_tools` on the agent socket | Connect = available; disconnect = auto-unregister |

## Rules that matter mid-task

- Cannot shadow core tool names (`bash`, `read`, `write`, `edit`, `ls`, `grep`, `find`, `docs`, `recall`, `spawn`, `agent_cmd`, `workflow`, …).
- `--disable-tool` denylist lasts for the process; UDS cannot usefully re-add those names for the LLM.
- `get_extensions` lists **tool** names (e.g. `web_search`), not internal package labels.
- UDS tool default timeout ~30s; disconnect mid-call → error result.

## Web (if enabled)

- Prefer Brave when `tools.web.brave` + API key (`QUECTO_TOOLS_WEB_BRAVE_API_KEY` or config); else DuckDuckGo.
- `web_fetch` is for full page text after search — respect tool schema limits.

## See also

- Entry: `docs {"name":"quick-start"}`
- Full human reference: `docs/extensions.md` in the repo
