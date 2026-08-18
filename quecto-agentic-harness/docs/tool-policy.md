# Tool policy persistence

Quecto exposes live tool access through the catalogue and `set_tool_policy` command. By default, a policy mutation changes the current session/profile overlay only. Clients that want an immediately applied choice to survive restart set `persist: true` on `set_tool_policy` (the TUI Ctrl+T modal applies immediately and does this when applying scopes). Queued `atNextTurnBoundary` policy changes remain live-session changes; persist them with an immediate request after the boundary if they should become defaults. The agent stores successful choices in the active config under `tools.policy.entries`, keyed by each catalogue entry's stable tool id.

Example:

```json
{
  "tools": {
    "policy": {
      "entries": {
        "tool.v1:native:21:quecto:official-tools:web_search": { "scope": "both" },
        "tool.v1:native:21:quecto:official-tools:python_lab": { "scope": "none" }
      }
    }
  }
}
```

Unknown or removed stable ids are safe: config load succeeds, the entry is kept in the file, and the running registry ignores/reports it until a matching tool is available again.

## Precedence

Effective availability is an intersection, never a union:

`runtime availability ∩ entrypoint default ∩ persisted tools.policy preference ∩ live profile overlay ∩ session/spawn restrictions`.

Persisted preferences are user defaults, not authority. They cannot widen startup ceilings such as `--disable-tool`, spawn inherited restrictions, read-only child restrictions, or runtime absence. If a persisted entry asks for `both` but the session ceiling is `none`, the tool remains unavailable and the catalogue explains the restriction. Live `set_tool_policy` mutations can narrow the current session further; persisted entries are applied on process startup as the configured/profile baseline.
