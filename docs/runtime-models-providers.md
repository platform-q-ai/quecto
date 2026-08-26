# Runtime models and providers

Quecto has one **effective catalogue** of provider/model descriptors. The domain owns stable identities, capabilities, availability, and immutable generation snapshots. The application resolves source layers, answers queries and selections, refreshes sources, and publishes provider routing with the catalogue as one runtime generation. Infrastructure parses and persists external formats, discovers remote metadata, resolves credentials, and constructs concrete transports. CLI, UDS, and TUI consume projections of the application snapshot.

## Source precedence

Catalogue sources are resolved as ordered layers, lowest precedence first: built-in metadata, then user-owned `models.json`, then the composed runtime layer that carries credential- and adapter-derived availability. Later layers upsert earlier ones by stable `provider/model` identity and keep the earlier entry's position, so listing order is stable when a user overrides shipped metadata. A base source that fails to load is reported and skipped; the remaining layers still publish a coherent generation. The runtime layer is different: if the composed runtime cannot be built — a malformed `models.json`, a configuration the providers reject — resolution fails and the last valid generation is retained, because a catalogue without the routing it describes is not a usable generation.

Queries are derived views over that one snapshot, narrowing in order: `Known` (every entry), `Configured` (usable local configuration), `Available` (configured and backed by a transport adapter), `Runnable` (can run right now).

`configured` on a listed model means the runtime has what it needs to talk to that provider — a key in config, a credential in the store, an OAuth token, or an endpoint that supplies one — not merely that the entry declared a key or a base URL.

Two definitions of one route are a configuration error, not a precedence question: an `openai_compatible` endpoint pointing at a different base URL than the `models.json` entry of the same prefix is reported as a duplicate prefix rather than one silently winning.

## Add or correct model metadata

For an existing provider, add the model to `models.json` or use explicit catalogue refresh. A user override with the same stable `provider/model` identity wins over older metadata without recompiling Quecto. Do not add model tables to CLI or TUI code.

## Add a provider using an existing transport

A user can declare a provider and models in `models.json` when its protocol and authentication shape are already supported. Choose the existing transport, provide its endpoint and credential configuration, and use stable provider/model IDs. Catalogue data does not make an unsupported protocol runnable.

## Add a new transport or authentication flow

Implement the provider construction extension in `infrastructure/provider_runtime.rs` and the concrete adapter under `infrastructure/providers`. Translate external metadata into domain descriptors and derive structured availability. Do not introduce another registry, descriptor type, precedence policy, or consumer-side model list.

## Add a catalogue source

Implement infrastructure loading/refresh behind the application refresh and composition ports. Source output is domain descriptors; application resolution owns stable-ID precedence and generation publication. Network access belongs only to the explicit refresh operation, never ordinary reads or local reload.

## Consumer contract

Startup and reload use the same runtime composer. Listing and model selection read the same effective catalogue generation. Selecting a model records it and reports why it cannot currently run, rather than refusing the switch; the session keeps working with what the runtime can route. The TUI requests `/models` and never maintains fallback model metadata.
