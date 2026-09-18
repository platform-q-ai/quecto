# Models and providers (deep dive)

Use `set_model` / CLI `--model` with the tool and flag schemas you already have. This page is registry editing only.

## How the catalogue works

Quecto resolves one **effective catalogue** from ordered source layers — built-in metadata, then discovered (refresh-cached) models, then the user-owned `models.json` providers/models, then its `overrides` section. Later layers upsert earlier ones by stable `provider/model` id. Every surface (CLI listing, `set_model`, TUI) reads the same published snapshot; a malformed input degrades alone with a diagnostic and the last valid state is kept.

## Where config lives

- Default model: `agents.defaults.model` of the global `~/.quecto/config.json`, or of the repository overlay `./.quecto/config.json` merged over it (`quecto status` names both; see `docs {"name": "config"}`), using a qualified `provider/model` id, for example:
  ```json
  {"agents": {"defaults": {"model": "openai-oauth/gpt-5.6-sol"}}}
  ```
  Never hand-edit: `quecto config set agents.defaults.model '"provider/model"'` (this repository) or `--global`; see "Pin a default model for this repository" below.
- User registry: `~/.quecto/models.json` (do **not** edit harness source to add a model).
- API keys / OAuth tokens: credential store via `quecto auth` or env (`OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, …). Catalogue files carry credential *references* like `"apiKey": "$MY_KEY"` — never literal secrets (a literal in `overrides` is rejected).
- Valid edits hot-reload into a new catalogue generation — no restart of Quecto or the TUI needed.
- Reasoning effort is per model: a `models.json` model on an OpenAI-compatible provider (Fireworks, a local server) offers `low/medium/high` **only if its record declares `"reasoning": true`**; xAI Grok built-ins and OpenAI reasoning built-ins (`gpt-5.6-*`, `gpt-6-*`, and every `openai-oauth` model) carry their documented scales, while `openai-api` ids served over Chat Completions (`gpt-5.5`, mini/nano, the codex ids) have none; a model with no effort control advertises an empty `effortLevels` and refuses `set_effort`. Use `get_state`'s `effortLevels` — never guess a level from the provider name.

## Agent procedure

1. Read existing `~/.quecto/models.json` (or start from `{"providers": {}}`).
2. **Add a model** to an existing provider: append to that provider's `models` array (`id`, optional `name`, `contextWindow`, `maxTokens`).
3. **Add a provider** on a runnable transport (`api`: `openai-completions` or `anthropic-messages`) with `baseUrl` and a `$ENV` credential reference. `google-generative-ai` is recognized by the registry but not runnable in this build. A transport with no adapter lists the models as known-but-not-runnable with a structured reason — data cannot enable a protocol.
4. **Fix stale metadata** with the top-level `overrides` map, keyed by qualified id — patches any known entry in place (fields: `name`, `contextWindow`, `maxTokens`, `apiKey` reference):
   ```json
   {"overrides": {"openai-api/gpt-5.5": {"contextWindow": 999000}}}
   ```
5. Ensure auth exists for that provider, then `set_model` to `provider/modelId` and verify with a tiny prompt. A model that cannot run is reported with the structured reason (missing credential, unsupported transport, unknown model) instead of a refused switch.
6. To pull a provider's remote model list into the discovered layer: `quecto models discover <provider-key>` (OpenAI-compatible `/models` endpoints only; never rewrites `models.json`).

## Pin a default model for this repository

Run from the repository root (the overlay is discovered in the working directory only). The model must be a qualified `provider/model` id that lists as `configured`.

```
quecto config set agents.defaults.model '"openai-api/gpt-5.6-luna"'
quecto config set agents.defaults.effort '"high"'                 # optional
quecto config get --effective agents.defaults.model               # verify: "openai-api/gpt-5.6-luna"
quecto status                                                     # verify: Overlay: … (trusted)  Model: …
```

New agents started in this directory start on it (their `get_state` reports the `model`); other repositories keep the global default; the global file is untouched. Your running session is not switched — `set_model` for that, or `set_model` with `"persist":"local"` to switch and pin in one step (`"global"` pins for every repository; a refused pin — untrusted overlay, symbolic link, explicit `--config`, bare id — changes nothing). Rollback: `quecto config unset agents.defaults.model` (and `agents.defaults.effort`); `--global` for the global file. Full runbook with preconditions and refusals: `docs {"name": "config"}`.

## See also

- Manual index: `docs {}`
- Full human reference: `docs/runtime-models-providers.md` in the repo
