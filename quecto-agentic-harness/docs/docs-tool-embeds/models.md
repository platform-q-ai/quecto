# Models and providers (runbook): credentials, default model, registry

Three jobs: store a credential, pin the default model (per repo or global),
add a provider/model to the registry. `set_model` / `--model` switch one
session and are not covered here.

## How the catalogue works

One **effective catalogue** is built from ordered layers — built-in metadata,
discovered (refresh-cached) models, the user-owned `<base_dir>/models.json`
providers/models, then its `overrides` — later layers upsert earlier ones by
stable `provider/model` id. Built-in provider slots are `openai-api`,
`openai-oauth`, `anthropic-api`, `anthropic-oauth` (bare `openai/…` is not a
slot). A `models.json` provider's key is its slot. Every surface (CLI, UDS
`set_model`, TUI `/model`) reads the same snapshot; a malformed file keeps
the last valid state and logs why. Valid edits hot-reload before the next
turn — never tell the user to restart.

## Preconditions

- `quecto version` works; `quecto status` exits 0 (its `Config:` line is the global file; `providers`, `credentials.json` and `models.json` all live in that base dir — `QUECTO_BASE_DIR` moves them; `Workspace:` does not move).
- For a repo default: you are in the repository root and `quecto status` shows `Overlay: none` or `(trusted)` (see `docs {"name": "config"}` for `(untrusted)`).
- The model id is qualified: `provider/model` (`openai-api/gpt-5.6-luna`, `anthropic-api/claude-…`, `<models.json key>/<id>`). The writer does not check the id against the catalogue, so prove it runs (Verify).
- Reasoning effort is per model: OpenAI reasoning built-ins (`gpt-5.6-*`, `gpt-6-*`, every `openai-oauth` model) accept `none, low, medium, high, xhigh`; Anthropic built-ins `low, medium, high, max`; xAI Grok `low, medium, high` (`grok-4.6`: `xhigh` too); `openai-api` Chat Completions ids (`gpt-5.5`, mini/nano, codex) have no effort control (`set_effort` is refused); a `models.json` model has one only with `"reasoning": true`. The file validator (`config set … agents.defaults.effort`) accepts any of `none, low, medium, high, xhigh, max` for every model; only the model rejects an unsupported level, at run time. Read `get_state`'s `effortLevels`, never guess.

## Do

**1. Credential** (writes `<base_dir>/credentials.json`, 0600; never a config file):

```
quecto auth login --provider openai --token sk-proj-…        # → Credential stored for openai
quecto auth login --provider anthropic --token sk-ant-…      # → Credential stored for anthropic
```

From an agent, **always pass `--token`**: without it (or with `--oauth`,
which is the same thing) the command starts the browser OAuth flow and
blocks until the callback arrives on `localhost:1455` — a tool call never
returns from it. `--device-code` (openai and custom `models.json` providers only; anthropic
refuses it) prints a URL and a code, then waits — both flows are for a human
at a terminal. Ask the user for
the key; do not echo it back, do not write it into any file yourself. Environment variables `OPENAI_API_KEY` / `ANTHROPIC_API_KEY`
also work for one process. The store takes priority over
`providers.openai.api_key` in the global file.

**2. Default model** — this repository (writes `./.quecto/config.json`, trusted):

```
quecto config set agents.defaults.model '"openai-api/gpt-5.6-luna"'   # → set agents.defaults.model in /repo/.quecto/config.json (created) (trusted)
quecto config set agents.defaults.effort '"high"'                     # optional
```

Every repository (writes `<base_dir>/config.json`):

```
quecto config set --global agents.defaults.model '"openai-api/gpt-5.6-luna"'
```

From a running session the same record is `set_model {"model":"openai-api/gpt-5.6-luna","persist":"local"}` (`"global"` for the global file; `set_effort` likewise) — it switches this session *and* pins; the TUI `/model` selector offers it with Tab. A refused pin (untrusted overlay, symbolic link, explicit `--config`, bare id) changes nothing.

**3. Registry** (`<base_dir>/models.json`; start from `{"providers": {}}`; never edit harness source):

- Add a model to an existing provider: append to its `models` array (`id`, optional `name`, `contextWindow`, `maxTokens`, `reasoning`).
- Add a provider on a runnable transport: `"api": "openai-completions"` or `"anthropic-messages"`, `baseUrl`, `"auth": {"mode":"apiKey","apiKey":"$MY_KEY"}` (an `$ENV` reference — a literal secret in `overrides` is rejected) or `{"mode":"oauth","oauthProvider":"openai"|"anthropic"}`. `google-generative-ai` is recognised but not runnable in this build.
- Fix stale metadata: top-level `"overrides": {"openai-api/gpt-5.5": {"contextWindow": 999000}}`.
- Refresh a provider's model list from its OpenAI-compatible `/models` endpoint: `quecto models discover <provider-key>` — `<provider-key>` is a `models.json` provider with `"api": "openai-completions"` and a `baseUrl`; built-in slots are refused (`no refreshable catalogue source named 'openai-api'`). It rewrites only that provider's `models` array, atomically; `--watch --interval 3600` keeps it fresh.

## Verify

```
quecto auth status                                     # → Credentials:\n  openai (token) — active
quecto config get --effective agents.defaults.model    # → "openai-api/gpt-5.6-luna"
quecto status                                          # → Overlay: … (trusted)  Model: openai-api/gpt-5.6-luna  Effort: high
quecto agent --no-session -m "Reply with exactly OK"   # prints the reply only: OK
```

The last line runs one model call on the pinned model from this directory
and proves the id (and any effort) is runnable with the stored credential;
skip it if the user does not want a model call. Other
repositories keep the global default (`quecto config get --global
agents.defaults.model` is unchanged by a repo pin). Your own running session
is not switched: `set_model` for that. A registry edit shows up in the next
turn's `/model` list or a child's `get_state.model`.

## Rollback

```
quecto auth logout --provider openai                   # → Credential removed for openai
quecto config unset agents.defaults.model              # the global default applies again
quecto config unset agents.defaults.effort
quecto config unset --global agents.defaults.model     # the built-in default applies again
```

Registry: remove the entry from `models.json` (keep valid JSON); the next
turn reloads it.

## If it fails

| Symptom | Cause | Fix |
|---|---|---|
| `quecto auth status` → `no credentials stored` after a login | a different base dir | `echo $QUECTO_BASE_DIR`; log in with the same environment the agents run in |
| `quecto status` → `OpenAI API: not set` although `auth status` is active | `status` reports the global file's `providers.openai.api_key` only | nothing to fix; trust `auth status` |
| one-shot run fails with a credential/401 error | wrong slot (`openai-api` needs an API key, `openai-oauth` an OAuth login) or expired OAuth (`auth status` → `expired`) | pin the slot that matches the credential, or have the user run `quecto auth login … --oauth` at a terminal |
| one-shot run reports an unknown model | id not in the catalogue for that slot | use an id the provider serves; for a custom provider add it to `models.json` first |
| `refusing to write …: the result is not a valid configuration: invalid effort level 'x' …` | not one of `none, low, medium, high, xhigh, max` | pick one; the file is unchanged |
| `set_effort` refused, `effortLevels: []` | the model has no effort control | leave effort unset |
| a `models.json` provider is listed as not runnable | transport without an adapter, or credential reference unresolved | use `openai-completions`/`anthropic-messages`; export the `$ENV` the `apiKey` names |
| `/model` still lacks the new entry | malformed `models.json` (last-good kept) | `jq . <base_dir>/models.json` to find the error |
| `quecto auth login` never returns | run without `--token` (browser OAuth flow waiting for a callback) | kill it; rerun with `--token`, or let the user finish it at a terminal |

## See also

- Manual index: `docs {}` · setup index: `docs {"name": "setup"}`
- Overlay and trust details: `docs {"name": "config"}`
- Full human reference: `docs/runtime-models-providers.md` in the repo
