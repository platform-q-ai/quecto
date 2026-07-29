# Models and providers (deep dive)

Use `set_model` / CLI `--model` with the tool and flag schemas you already have. This page is registry editing only.

## Where config lives

- User registry: `~/.quecto/models.json` (do **not** edit harness source to add a model).
- API keys / OAuth tokens: credential store via `quecto auth` or env (`OPENAI_API_KEY`, `ANTHROPIC_API_KEY`, …) — not inside `models.json` secrets if the project keeps them separate.
- Changes are picked up on reload / next model UI / `set_model` — prefer hot path over “restart everything” unless reload fails.

## Agent procedure

1. Read existing `~/.quecto/models.json` (or create from documented schema in `docs/runtime-models-providers.md`).
2. Add provider + model entries with correct `id` / wire protocol fields.
3. Ensure auth exists for that provider.
4. `set_model` to `provider/modelId` (or equivalent) and verify with a tiny prompt.

## See also

- Entry: `docs {"name":"quick-start"}`
- Full human reference: `docs/runtime-models-providers.md` in the repo
