# Runtime models and providers

Quecto's runtime model registry lives at `~/.quecto/models.json`. It is the user/community extension point for adding model metadata and API-key providers **without recompiling quecto**. This doc is the single source of truth for agents that need to add, edit, or explain provider/model setup.

> **Agent quick start:** If a user asks you to add a provider/model, edit `~/.quecto/models.json` (never edit source code for this). Use the schema below. The change is hot-reloaded on the next turn, `/model` open, or `set_model` — do not tell the user to restart.

## The file and how it is consumed

`~/.quecto/models.json` is read by the `ModelRegistry` parser (`src/infrastructure/model_registry.rs`) and turned into runtime providers by `build_agent_provider` (`src/interface/cli/agent_provider.rs`). You do not need to touch either file to add a provider — editing `models.json` is sufficient.

**How hot reload works (mechanics):**

1. The reload gate (`src/infrastructure/reload.rs`) watches both `~/.quecto/config.json` and `~/.quecto/models.json` by mtime + length + content hash.
2. On a poll, if metadata is unchanged it does **not** read the file (cheap). If metadata changed, it reads and hashes; if the hash changed, it rebuilds the provider router.
3. Poles happen automatically before each prompt, before `set_model`, when `/model` is opened (TUI re-requests the list), and on an explicit UDS `reload`.
4. Reload is **fail-safe**: if the new file is malformed, the last-good provider router stays active and a warning is logged — the session does not crash.
5. Because quecto-tui talks to the agent over UDS, it never needs its own restart either.

So: **edit the file, save, send the next prompt or reopen `/model` — the new provider/model is live.**

## Where keys go (do not mix these up)

| Provider kind | Where the API key lives | Example |
|---|---|---|
| Built-in `openai-api` / `anthropic-api` | `~/.quecto/config.json` `providers.openai.api_key` / `providers.anthropic.api_key`, or env `QUECTO_PROVIDERS_OPENAI_API_KEY` / `QUECTO_PROVIDERS_ANTHROPIC_API_KEY` | `"providers": {"openai": {"api_key": "sk-..."}}` |
| Community / custom API providers | `~/.quecto/models.json` under the provider's `auth.apiKey` | `"auth": {"mode":"apiKey","apiKey":"$FIREWORKS_API_KEY"}` |
| OAuth providers | Kernel credential store via `quecto auth login`; referenced from `models.json` by `auth.oauthProvider` | `"auth": {"mode":"oauth","oauthProvider":"anthropic"}` |

**API key interpolation:** `auth.apiKey` supports `$ENV` and `${ENV}` interpolation, resolved when the registry loads/reloads. Use `$$` for a literal dollar. Prefer env interpolation over committing literal keys.

## Explicit auth modes

Provider keys are auth-specific. Do not overload a single `openai` or `anthropic` key to mean both OAuth and API key — that can silently switch billing mode.

Built-in provider names:

- `openai-api` — OpenAI API key (`QUECTO_PROVIDERS_OPENAI_API_KEY` or config).
- `openai-oauth` — OpenAI OAuth credential from `quecto auth login openai`.
- `anthropic-api` — Anthropic API key (`QUECTO_PROVIDERS_ANTHROPIC_API_KEY` or config).
- `anthropic-oauth` — Anthropic OAuth credential from `quecto auth login anthropic`.

The `/model` selector surfaces auth as `[apiKey]` or `[oauth]` so the billing mode is visible before selection. Bare vendor prefixes (`openai/...`, `anthropic/...`) should not be used for new configs because they hide billing mode.

## Registry schema

```json
{
  "providers": {
    "provider-key": {
      "api": "openai-completions",
      "baseUrl": "https://example.com/v1",
      "auth": { "mode": "apiKey", "apiKey": "$EXAMPLE_API_KEY" },
      "allowRemoteHttp": false,
      "models": [
        {
          "id": "provider/model/id",
          "name": "Display Name",
          "contextWindow": 128000,
          "maxTokens": 16384,
          "input": ["text"],
          "reasoning": false,
          "cost": { "input": 0.0, "output": 0.0 }
        }
      ]
    }
  }
}
```

Supported wire protocols today:

- `openai-completions`
- `anthropic-messages`

`google-generative-ai` is reserved in the registry parser but provider construction is not implemented yet.

API keys support `$ENV` and `${ENV}` interpolation. Use `$$` for a literal dollar.

## API-key provider example

```json
{
  "providers": {
    "fireworks": {
      "api": "openai-completions",
      "baseUrl": "https://api.fireworks.ai/inference/v1",
      "auth": { "mode": "apiKey", "apiKey": "$FIREWORKS_API_KEY" },
      "models": [
        { "id": "accounts/fireworks/models/glm-5p2", "name": "GLM 5.2" }
      ]
    }
  }
}
```

Use it as:

```text
/model fireworks/accounts/fireworks/models/glm-5p2
```

The provider key is `fireworks`; the model id is the full slashful tail.

## OAuth-backed provider example

OAuth stays kernel-owned. Community data may reference only kernel-known OAuth identities: `openai` and `anthropic`.

```json
{
  "providers": {
    "anthropic-oauth": {
      "api": "anthropic-messages",
      "auth": { "mode": "oauth", "oauthProvider": "anthropic" },
      "models": [
        { "id": "claude-opus-4-8", "name": "Claude Opus 4.8 (OAuth)" }
      ]
    }
  }
}
```

Setup:

```bash
quecto auth login anthropic
```

Then select:

```text
/model anthropic-oauth/claude-opus-4-8
```

## Same vendor, both billing modes

```json
{
  "providers": {
    "anthropic-api": {
      "api": "anthropic-messages",
      "baseUrl": "https://api.anthropic.com",
      "auth": { "mode": "apiKey", "apiKey": "$ANTHROPIC_API_KEY" },
      "models": [{ "id": "claude-opus-4-8", "name": "Claude Opus 4.8 (API)" }]
    },
    "anthropic-oauth": {
      "api": "anthropic-messages",
      "auth": { "mode": "oauth", "oauthProvider": "anthropic" },
      "models": [{ "id": "claude-opus-4-8", "name": "Claude Opus 4.8 (OAuth)" }]
    }
  }
}
```

This gives two explicit selector entries and no silent fallback between API and OAuth.

## How to edit `models.json` (agent procedure)

When a user asks you to add or change a provider/model, follow this exactly:

1. **Read** `~/.quecto/models.json` first. It is a single JSON object with a top-level `providers` map; preserve existing entries and keys.
2. **Add or edit** one provider block under `providers`. Pick a descriptive, auth-specific provider key (e.g. `fireworks`, `anthropic-api`, `anthropic-oauth`). The key is the routing prefix users type before the model id.
3. **Set `api`** to the correct wire protocol: `openai-completions` or `anthropic-messages`.
4. **Set `auth`:**
   - API key → `"auth": { "mode": "apiKey", "apiKey": "$ENV_VAR" }` (preferred) or a literal key.
   - OAuth → `"auth": { "mode": "oauth", "oauthProvider": "anthropic" | "openai" }`. OAuth can only reference kernel-known identities; anything else is rejected. Do not set a custom `baseUrl` for OAuth — it is constrained to the canonical provider host.
5. **Set `baseUrl`** for API-key providers that are not the built-in OpenAI/Anthropic endpoints.
6. **List `models`** with `id` (the exact id the provider expects, may contain `/`), a human `name`, and optional metadata (`contextWindow`, `maxTokens`, `input`, `reasoning`, `cost`).
7. **Save the file.** Do not restart quecto or quecto-tui. Tell the user to open `/model` or send the next prompt — the change is live.
8. If the user reports a model is missing from `/model`, confirm the JSON is valid and the `auth` block is correct; a malformed file keeps the last-good router and silently ignores the new entry.

**Do not** edit source code to add providers/models. **Do not** tell users to restart. **Do not** use bare `openai/...` or `anthropic/...` for new configs — use the explicit `*-api`/`*-oauth` keys.

## Internal agent guidance

1. Prefer `models.json` for community/runtime additions.
2. Use explicit auth-specific provider keys (`*-api`, `*-oauth`).
3. OAuth references must be one of the kernel-known identities (`openai`, `anthropic`); otherwise the provider must be an API-key/sidecar provider.
4. Changes are hot-loaded on consume; no restart is required.
5. When in doubt about the schema, read this doc with `docs {"name": "models-providers"}` rather than guessing.
