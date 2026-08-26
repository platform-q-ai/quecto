# Runtime models and providers

Quecto has one **effective catalogue** of provider/model descriptors. The domain owns stable identities, capabilities, availability, and immutable generation snapshots. The application resolves source layers, answers queries and selections, refreshes sources, and publishes provider routing with the catalogue as one runtime generation. Infrastructure parses and persists external formats, discovers remote metadata, resolves credentials, and constructs concrete transports. CLI, UDS, and TUI consume projections of the application snapshot.

## Layer ownership

One authority owns each concern; nothing outside its layer re-derives it. Contributions extend the owning layer — creating another authority (a second registry, a consumer-side model table, an interface-level capability heuristic) is a defect, not a shortcut.

| Layer | Owns | Never does |
|---|---|---|
| `domain` (`domain/catalogue.rs`) | Stable provider/model identities, capability metadata (limits, cost, the reasoning-effort vocabulary), availability semantics, immutable snapshot generations | I/O, parsing external formats |
| `application` (`application/catalogue.rs`, `catalogue_refresh.rs`, `provider_runtime.rs`) | Source-layer precedence and the resolve/merge, queries and selection, refresh orchestration, publishing catalogue + routing as one generation | Reading files or the network directly |
| `infrastructure` (`infrastructure/catalogue_registry.rs`, `catalogue_inputs.rs`, `catalogue_discovery.rs`, `providers/`) | Parsing `models.json` and discovery caches into domain descriptors, credential resolution, concrete transport adapters | Deciding precedence, defining canonical types, inferring capabilities |
| `interface` (`interface/catalogue_runtime.rs`, CLI/UDS/REPL, TUI) | Composing the layers at process entry points and rendering projections of the published snapshot | Parsing catalogue data, merging sources, caching its own model metadata |

## Source precedence

Catalogue sources are resolved as ordered layers, lowest precedence first: built-in metadata, then discovered (refresh-cached) models, then the user-owned `models.json` provider/model declarations, then the user's stable-ID `overrides` section. Later layers upsert earlier ones by stable `provider/model` identity and keep the earlier entry's position, so listing order is stable when a user overrides shipped metadata. A source that fails to load is reported and degrades to its own last successfully loaded entries on that store (or contributes nothing if it never loaded), so the remaining layers still publish a coherent generation and one broken input — a malformed `models.json`, a corrupt discovery cache — never freezes unrelated valid updates. A provider block the parse must skip (an unknown transport per AC3, or an unknown auth mode) degrades alone with a per-record diagnostic instead of failing the file. The runtime layer is different: if the composed runtime cannot be built — a malformed `models.json`, a configuration the providers reject — resolution fails and the last valid generation is retained, because a catalogue without the routing it describes is not a usable generation.

Queries are derived views over that one snapshot, narrowing in order: `Known` (every entry), `Configured` (usable local configuration), `Available` (configured and backed by a transport adapter), `Runnable` (can run right now).

`configured` on a listed model means the runtime has what it needs to talk to that provider — a key in config, a credential in the store, an OAuth token, or an endpoint that supplies one — not merely that the entry declared a key or a base URL.

Two definitions of one route are a configuration error, not a precedence question: an `openai_compatible` endpoint pointing at a different base URL than the `models.json` entry of the same prefix is reported as a duplicate prefix rather than one silently winning.

## User extension surface (`models.json`)

The user-owned `~/.quecto/models.json` (the harness base directory) is the extension surface. It supports three data-only operations, none of which require recompiling or restarting Quecto:

### Add a model to an existing provider

```json
{"providers": {"openai-api": {"api": "openai-completions",
  "models": [{"id": "gpt-5.5-preview", "name": "GPT 5.5 Preview",
              "contextWindow": 400000, "maxTokens": 65536}]}}}
```

### Add a provider on an existing transport

Choose a supported transport (`openai-completions`, `anthropic-messages`, `google-generative-ai`), provide its endpoint, and reference the credential from the environment — catalogue files carry credential *references*, never key material:

```json
{"providers": {"my-gateway": {"api": "openai-completions",
  "baseUrl": "https://gw.example/v1", "apiKey": "$MY_GATEWAY_KEY",
  "models": [{"id": "custom-model"}]}}}
```

A provider block declaring a transport with no adapter in this build is still listed: its models are *known but not runnable*, with a structured unsupported-transport reason naming the declared transport. Catalogue data does not make an unsupported protocol runnable, and the rest of the file keeps working.

### User overrides: patch a model's metadata by stable ID

The top-level `overrides` section patches an existing entry — built-in, discovered, or user-declared — by qualified `provider/model` id. Only the declared fields change; the entry is replaced in place, never duplicated:

```json
{"overrides": {"openai-api/gpt-5.5": {"name": "My 5.5",
  "contextWindow": 999000, "maxTokens": 32768,
  "apiKey": "$MY_OPENAI_KEY"}}}
```

Supported override fields: `name`, `contextWindow`, `maxTokens`, `apiKey` (credential reference only). An override whose `apiKey` is a literal secret, or whose target is not a known model, is rejected with a per-record diagnostic; the rest of the catalogue still publishes.

### Secrets

The `overrides` surface accepts only `$ENV`-style credential references; a literal secret is a structured error, and a reference to an unset or empty environment variable is also rejected with a diagnostic (the base credential is kept) rather than silently clobbering a working key. The top-level `overrides` section patches any known entry by qualified id, including known-but-unrunnable unsupported-transport declarations (metadata fields only). The legacy provider-level `apiKey` field continues to accept literal values for compatibility with existing `models.json` files (which keep working unchanged), but references are the documented, recommended form everywhere.

### Hot reload

Reload is pull-based (ADR-0002): every read surface (CLI listing, UDS `/models`, TUI projection via UDS) resolves through the same publish path, re-reading `models.json` and re-publishing a new immutable generation, so a valid edit is visible without restarting Quecto or the TUI, and no network is touched. A malformed edit retains the user layers' last valid contribution — never a partial publish of a half-parsed file — and surfaces the parse error to CLI/UDS/TUI diagnostics, while the other source layers keep publishing their own updates.

## Add or change domain metadata

Built-in model knowledge — identities, context windows, costs, the effort vocabulary — is domain metadata: it lives in the domain/built-in registry table and flows into every surface through the published snapshot. To correct or extend it, change the built-in data (or ship a user override); never patch a consumer. If a capability is not yet represented, add a field to the domain capability type and project it — do not infer it downstream, because that inference becomes another authority the snapshot cannot correct.

## Add a new transport or authentication flow

Implement the provider construction extension in `infrastructure/provider_runtime.rs` and the concrete adapter under `infrastructure/providers`. Translate external metadata into domain descriptors and derive structured availability. Do not introduce another registry, descriptor type, precedence policy, or consumer-side model list — any of these is another authority competing with the published snapshot, and convergence tests will fail it.

## Add a catalogue source

Implement infrastructure loading/refresh behind the application refresh and composition ports. Source output is domain descriptors; application resolution owns stable-ID precedence and generation publication. Network access belongs only to the explicit refresh operation, never ordinary reads or local reload.

## Consumer contract

Startup and reload use the same runtime composer. Listing and model selection read the same effective catalogue generation. Selecting a model records it and reports why it cannot currently run, rather than refusing the switch; the session keeps working with what the runtime can route. The TUI requests `/models` and never maintains fallback model metadata. Capability metadata — including each model's reasoning-effort vocabulary (`effortLevels`) — travels inside the listing and session-state payloads from the canonical snapshot; no consumer infers capabilities from provider or model names.
