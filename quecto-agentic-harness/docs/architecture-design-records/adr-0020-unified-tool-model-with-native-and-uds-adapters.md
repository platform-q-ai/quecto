# ADR-0020 — Unified Tool Model with Native and UDS Delivery Adapters

**Status:** Accepted.

**Implementation status (issue #1276):**

- **Phase 1–2 (landed):** `ToolDescriptor` / `ToolSource` / `ToolAvailability`,
  runtime enable/disable, and core FS/exec/search/docs construction via
  `build_official_tool_extensions`. UDS tools share the descriptor/policy path.
- **Phase 3 (landed):** Remaining official/default model-callable tools are
  supplied through bundled native provider seams:
  - `build_session_tool_extensions` → `recall`
  - `build_agent_control_tool_extensions` → `spawn` + `agent_cmd` (+ live handles)
  - `build_workflow_tool_extension` / `register_workflow_tool` → `workflow`
  - `build_native_extensions` → config-gated web tools collected by the legacy
    `ExtensionRegistry` package aggregator, then installed via bundled-native
    `registry.register(...)` / `register_bundled_native_extension_tools()`
    rather than runtime `register_runtime_tool`
  - `register_bundled_native_tools` / bundled-native registration for
    non-unloadable official registration
  Composition roots consume these providers through the shared
  `interface::tool_runtime` builder. CLI/UDS and REPL use that common pipeline;
  entrypoint differences such as REPL's narrower model-visible surface are
  represented as policy state rather than provider omission.
  `DocsTool::with_spawned` remains transitional.
- **Phase 3 additive catalogue state:** `ToolCatalogueEntry` records richer
  TUI/API-ready state without adding persisted profile UX: registered versus
  model-visible/effective availability, entrypoint/configured/profile/session
  placeholders, explicit restriction reasons, runtime lifecycle, provider/owner,
  availability, and coarse health. Existing descriptor and protocol surfaces stay
  backwards-compatible while consumers migrate to catalogue state.
- **Still open / deferred follow-ups:** legacy lifecycle API naming cleanup,
  protocol/TUI migration to consume rich catalogue state, persisted profile
  policy UX, full parent/child profile policy rewrite, intentional behaviour
  changes such as REPL parity, and WASM.

## Context

Quecto exposes model-callable tools from multiple places:

- official built-in Rust tools such as filesystem, search, exec, docs, and
  agent-control tools;
- bundled native extensions that group official Rust tools and prompt snippets;
- UDS extension clients that can register tools at runtime.

Historically, parts of the code and user-facing protocol used the word
"extension" to mean both a tool-delivery mechanism and a tool class. That
ambiguity makes policy, TUI inspection, parent/child/custom profile behaviour,
and third-party extensibility harder to reason about.

Issue #1276 established these principles:

1. Quecto has one model-callable tool model and one policy pipeline.
2. All official/default model-callable tools are bundled native extensions.
3. Third-party runtime tools use UDS.
4. Native and UDS are delivery adapters, not separate tool models.
5. Every registered tool is runtime configurable on/off without restarting
   Quecto.
6. Parent, child, and future custom profiles own tool availability; tools do
   not inspect roles.
7. TUI/application callers depend on descriptors and policy state, never on
   concrete tool implementations.
8. Native Rust additions/upgrades require rebuilding Quecto; UDS tools are
   runtime installable; WASM is not part of the design for now.

## Decision

Quecto will treat every model-callable capability as a `Tool` registered in a
single registry/catalogue. Delivery adapters attach metadata and lifecycle
behaviour around that same tool model instead of creating parallel registries or
execution paths.

Each registered tool has a descriptor containing at least:

- the model-facing `ToolDefinition`;
- a `source` identifying the delivery adapter, for example
  `bundled-native` or `uds`;
- an `owner` identifying the profile/policy owner, for example
  `quecto:official-tools` or `uds:runtime`;
- runtime availability, initially `enabled` or `disabled`.

Additive catalogue entries extend descriptors with non-breaking state for future
TUI/API consumers: stable id, provider id, lifecycle (`bundled` versus
`runtime-loadable`), default/configured/profile/session placeholders,
restriction reason, effective enabled state, and health. Placeholder profile
fields remain `None` until the persisted profile UX lands.

The descriptor is a boundary object for policy and UI. The TUI and CLI protocol
should read descriptors rather than import concrete Rust tool types or infer
behaviour from native/UDS implementation details.

Official/default tools are considered bundled native tools: compiled into the
binary, versioned with Quecto, and installed or upgraded by rebuilding/releasing
Quecto. UDS tools remain dynamically installable at runtime and communicate over
the length-prefixed UDS protocol. Native and UDS are therefore different
delivery adapters with different operational constraints, but they feed the
same catalogue, policy, and execution interfaces.

Runtime policy changes must not unregister tools merely to hide them from the
model. Disabling a tool keeps its descriptor registered, removes it from
model-visible definitions, and rejects new executions through the common tool
execution path. Startup `--disable-tool` restrictions use this descriptor-preserving
runtime policy and also reserve every named tool against future UDS/runtime
registration for the process lifetime. Re-enabling ordinary runtime-disabled tools
restores model visibility without restarting Quecto; session startup restrictions
remain denied to future registration unless a future privileged policy surface
explicitly changes that contract.

Profile-level decisions belong outside concrete tool implementations. Parent,
child, and future custom profiles may decide which descriptors are enabled, but
tools should not inspect their caller role to self-disable.

## Consequences

- There is one place to answer "what tools exist?", "what is model-visible?",
  and "what policy state applies?".
- Existing behaviours such as denylisted tool names, shadowing protection,
  extension unregister, guards, and session-key propagation continue to operate
  on the same registry.
- Disabled tools remain discoverable to policy/UI while not being exposed to the
  model and not executing work.
- UDS tools gain the same descriptor surface as native tools, including source,
  owner, and availability.
- Protocol/UI consumers should progressively migrate from extension-name-only
  views to descriptor-driven views.
- Native Rust tool changes still require compile/release cycles; runtime
  extensibility remains the job of UDS.
- WASM is intentionally excluded until a separate decision revisits it.

## Migration plan

1. Introduce descriptor metadata and runtime availability in the existing tool
   registry without changing the public execution model.
2. Add a UDS registration seam that records UDS source/ownership while
   preserving existing UDS tool registration semantics.
3. Expose descriptors through application/domain ports for protocol and TUI
   callers.
4. Move all official/default tool construction behind bundled native extension
   providers while preserving current tool names, schemas, guards, denylist,
   and child/parent defaults.
5. Add explicit profile policy owners for parent, child, and custom profiles;
   make profile policy mutate descriptor availability rather than asking tools
   to inspect roles.
6. Complete protocol/TUI migration to descriptor/policy views and retain
   backwards-compatible fields where necessary.

## Alternatives considered

- **Keep native tools and UDS tools as separate models.** Rejected: creates two
  policy paths, makes UI inspection brittle, and obscures common behaviour.
- **Use unregister/remove to disable tools.** Rejected: it loses descriptor
  state and cannot reliably re-enable without reconstructing/restarting.
- **Let tools self-disable based on parent/child role.** Rejected: violates
  clean architecture by pushing policy decisions into concrete implementations.
- **Support WASM as another runtime adapter now.** Rejected: no current need;
  UDS already covers runtime installability and keeps the scope bounded.
