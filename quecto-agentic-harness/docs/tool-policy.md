# Tool policy persistence

Quecto exposes live tool access through the catalogue and `set_tool_policy` command. By default, a policy mutation changes the current session/profile overlay only. Clients that want a successful choice to survive restart set `persist: true` on `set_tool_policy` (the TUI Ctrl+T modal does this when applying scopes). Immediate requests write successful choices when they apply; queued `atNextTurnBoundary` requests write successful choices when the boundary drains. Requests without `persist: true` remain live-session overlays only. The agent stores durable choices in the active config under `tools.policy.entries`, keyed by each catalogue entry's stable tool id.

Example:

```json
{
  "tools": {
    "policy": {
      "entries": {
        "tool.v1:bundled-native:3:web:web_search": { "scope": "both" },
        "tool.v1:bundled-native:21:quecto:official-tools:swarm": { "scope": "none" }
      }
    }
  }
}
```

Set one entry with the whole stable id as the last key (everything after `tools.policy.entries.` is that one key; a key that is not a stable id is refused): `quecto config set tools.policy.entries.tool.v1:bundled-native:21:quecto:official-tools:swarm '{"scope":"none"}'`.

Unknown or removed stable ids are safe: config load succeeds, the entry is kept in the file, and the running registry ignores/reports it until a matching tool is available again.

## Precedence

Effective availability is an intersection, never a union:

`runtime availability ∩ entrypoint default ∩ persisted tools.policy preference ∩ live profile overlay ∩ session/spawn restrictions`.

Persisted preferences are user defaults, not authority. They cannot widen startup ceilings such as `--disable-tool`, spawn inherited restrictions, read-only child restrictions, or runtime absence. If a persisted entry asks for `both` but the session ceiling is `none`, the tool remains unavailable and the catalogue explains the restriction. Live `set_tool_policy` mutations can narrow the current session further; persisted entries are applied on process startup as the configured/profile baseline. An explicit UDS/API `reload` reparses `tools.policy.entries`, reapplies that persisted baseline, and clears live-only AgentLoop overlays so edits/removals in config become visible without restarting.

A persisted entry whose stable id matches no tool registered on this entrypoint is kept and ignored. What happens at start-up depends on the id (#2217): a bundled tool this entrypoint does not build (`workflow` on the one-shot CLI) is kept quietly for the entrypoints that do; a retired bundled tool (`python_lab`, removed in #1684) is ignored quietly and its entry is safe to delete; an id in a namespace whose tools register after start-up (`tool.v1:uds:` for a UDS extension, `tool.v1:runtime:`) is kept quietly and applies when the tool registers (logged at debug); any other `tool.v1:bundled-native:` id, or a key that is not a stable id at all, is most likely a typo whose restriction never applies, and prints `WARNING: tools.policy: no tool has stable id '<id>' …` on stderr, naming `tools.policy.entries` as the place to fix it (a `reload` logs the same warning).

## Inheritance by spawned children

A spawned child receives its parent's inherited policy snapshot: each tool in the parent's catalogue, keyed by stable id, with the parent's effective scope. The snapshot is a ceiling. A recorded scope caps the child's tool and cannot be widened. A tool the snapshot does not record is closed to the child, including any tool registered late or over UDS. Every refresh of the snapshot (startup, `reload`, a live `set_tool_policy`, a turn-boundary drain, a UDS tool registering or leaving) passes through one registry step that records the parent's policy over entrypoint-only tools, so no refresh drops it.

Configured extensions (#2446) are the exception for tools a child registers late: each locally spawned child launches its own instance of every configured extension with `children: true`, so its tools register after start-up and may be named differently from the parent's (a tool named after `{agent_id}`, say). A configured extension's tools carry a stable id of their own extension (`tool.v1:uds:<n>:uds:extension:<name>:<tool>`), the same in parent and child, and the snapshot also records the extension itself, under `uds:extension:<name>`, with the widest scope among the parent's tools of that extension. A child's tool from its own instance takes, in order: the scope recorded for its stable id (a same-named tool keeps exactly the parent's scope, a denial included), then for its name, then the extension's entry. So a tool the parent's policy allows stays allowed in the child, and an extension's tools count as recorded when the parent's catalogue holds that extension's tools; when it holds none (the parent's instance is down, or was never configured), the child's are closed like any unrecorded tool. Keys that register late (UDS or runtime ids, extension entries) draw no start-up warning in the child. The child's list of extensions comes from the same snapshot (`extensions`): the parent's validated list, those marked `children`; the child ignores any `extensions` in its own config.

Entrypoint-only tools are the other exception. These are bundled tools that some entrypoints never build; today that is only `workflow`, because a one-shot CLI agent has no workflow runtime. If the snapshot does not record such a tool, the parent's entrypoint never built it, and that is not a denial: the child's own policy governs the tool. The ceilings differ in form: a UDS parent's children are capped by that parent's own workflow scope, while a CLI parent's children are capped only by its denials or its persisted `tools.policy` scope. In practice the outcome is the same: the children of a CLI parent get workflow as the children of a UDS parent do (a plain UDS child builds a dormant workflow tool; `workflow` or `workflow_spec` engages it).

A parent that could build workflow but did not closes it to its children, as it always did: a UDS agent started with `--no-workflow`, a swarm member, or an agent whose bound spec failed to load records `none`. A parent that denies the tool also records `none`: `--disable-tool workflow` (or a spawn's `disable_tools`) records it over any other entry. A persisted `tools.policy` entry records its scope when the tool was never built, never wider than a same-named tool's entry. Only a bundled-native registration may use a stable id in the `tool.v1:bundled-native:` namespace, so no UDS or runtime tool can pose as the bundled workflow tool.

A spawn that asks for workflow mode (`workflow`, `workflow_guards` or `workflow_spec`) is refused before launch if its `disable_tools` names the workflow tool, or if the parent's snapshot records workflow without child scope. A spawned child whose workflow engine was built but whose workflow tool is still hidden, for example because its own policy denies it, refuses to start, and the parent's `spawn` fails with the child's error. A spawned child whose engine was expected but could not be built, because its bound spec is missing or unreadable, also refuses to start, and the error names the load failure. A swarm member builds no engine by design and starts without workflow mode rather than refusing; a top-level agent whose spec fails to load starts without an engine, as before.
