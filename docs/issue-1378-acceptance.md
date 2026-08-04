# Issue #1378 acceptance criteria

- Each spawn mints a fresh hidden `AgentUuid`; display labels never key sockets, persisted child sessions, registry entries, parent/child edges, monitor/reaper, or await-dedupe.
- Reusing a display label after the previous agent exits creates a clean session with a different UUID/session key and no inherited context.
- Parent tools keep accepting display labels for live agents only and resolve them through domain policy.
- A second live agent with the same display label is rejected with a clear duplicate-name error.
- Exited/dead agents are not targetable by display label.
- Wire compatibility is preserved: `agent_id` remains the display label; additive `agent_uuid` and `display_name` fields are populated.
- TUI/API consumers key state by UUID while rendering display labels, with legacy fallback for older events.
- Protocol docs and capability matrix describe dual identity and live display-name targeting.
