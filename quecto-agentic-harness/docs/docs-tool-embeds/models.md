# Models and providers (deep dive)

Use `set_model` / CLI `--model` with the tool and flag schemas you already have. This page is registry editing only.

## How the catalogue works

Quecto resolves one **effective catalogue** from ordered source layers — built-in metadata, then discovered (refresh-cached) models, then the user-owned `models.json` providers/models, then its `overrides` section. Later layers upsert earlier ones by stable `provider/model` id. Every surface (CLI listing, `set_model`, TUI) reads the same published snapshot; a malformed input degrades alone with a diagnostic and the last valid state is kept.

## Where config lives

- User registry: `~/.quecto/models.json` (do **not** edit harness source to add a model).
- API keys / OAuth tokens: credential store via `quecto auth` or env (`OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, …). Catalogue files carry credential *references* like `"apiKey": "$MY_KEY"` — never literal secrets (a literal in `overrides` is rejected).
- Valid edits hot-reload into a new catalogue generation — no restart of Quecto or the TUI needed.

## Agent procedure

1. Read existing `~/.quecto/models.json` (or start from `{"providers": {}}`).
2. **Add a model** to an existing provider: append to that provider's `models` array (`id`, optional `name`, `contextWindow`, `maxTokens`).
3. **Add a provider** on a supported transport (`api`: `openai-completions`, `anthropic-messages`, `google-generative-ai`) with `baseUrl` and a `$ENV` credential reference. A transport with no adapter lists the models as known-but-not-runnable with a structured reason — data cannot enable a protocol.
4. **Fix stale metadata** with the top-level `overrides` map, keyed by qualified id — patches any known entry in place (fields: `name`, `contextWindow`, `maxTokens`, `apiKey` reference):
   ```json
   {"overrides": {"openai-api/gpt-5.5": {"contextWindow": 999000}}}
   ```
5. Ensure auth exists for that provider, then `set_model` to `provider/modelId` and verify with a tiny prompt. A model that cannot run is reported with the structured reason (missing credential, unsupported transport, unknown model) instead of a refused switch.
6. To pull a provider's remote model list into the discovered layer: `quecto models discover <provider-key>` (OpenAI-compatible `/models` endpoints only; never rewrites `models.json`).

## See also

- Entry: `docs {"name":"quick-start"}`
- Full human reference: `docs/runtime-models-providers.md` in the repo
