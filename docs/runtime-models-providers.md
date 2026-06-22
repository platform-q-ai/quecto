# Runtime models and providers

Quecto's runtime model registry lives at `~/.quecto/models.json`. It is the user/community extension point for adding model metadata and API-key providers without recompiling quecto.

## Hot reload contract

The agent watches both `~/.quecto/config.json` and `~/.quecto/models.json` through the runtime reload gate. Changes are consumed without restarting quecto or quecto-tui:

- opening `/model` asks the agent for a fresh model list;
- `set_model` polls reload before switching;
- each prompt polls reload before provider use;
- explicit UDS `reload` forces a rebuild.

Reload is fail-safe: if the new file is invalid, the last-good provider router remains active.

## Explicit auth modes

Provider keys are auth-specific. Do not overload a single `openai` or `anthropic` key to mean both OAuth and API key.

Built-in provider names:

- `openai-api` — OpenAI API key (`QUECTO_PROVIDERS_OPENAI_API_KEY` or config).
- `openai-oauth` — OpenAI OAuth credential from `quecto auth login openai`.
- `anthropic-api` — Anthropic API key (`QUECTO_PROVIDERS_ANTHROPIC_API_KEY` or config).
- `anthropic-oauth` — Anthropic OAuth credential from `quecto auth login anthropic`.

The selector surfaces auth as `[apiKey]` or `[oauth]`. Bare vendor prefixes should not be used for new configs because they hide billing mode.

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

## Internal agent guidance

When adding or documenting model/provider setup:

1. Prefer `models.json` for community/runtime additions.
2. Use explicit auth-specific provider keys (`*-api`, `*-oauth`).
3. Never suggest bare `openai/...` or `anthropic/...` for new configs.
4. OAuth references must be one of the kernel-known identities; otherwise the provider must be an API-key/sidecar provider.
5. Explain that changes are hot-loaded on consume; no restart is required.
