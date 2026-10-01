# Runtime models and providers

Quecto's runtime model registry lives at `~/.quecto/models.json`. It is the user/community extension point for adding model metadata and API-key providers **without recompiling quecto**. This doc is the single source of truth for agents that need to add, edit, or explain provider/model setup.

> **Agent quick start:** If a user asks you to add a provider/model, edit `~/.quecto/models.json` (never edit source code for this). Use the schema below. The change is hot-reloaded on the next turn, `/model` open, or `set_model` — do not tell the user to restart.

## The file and how it is consumed

`~/.quecto/models.json` is read by the `ModelRegistry` parser (`src/infrastructure/model_registry.rs`) and turned into runtime providers by `build_agent_provider` (`src/composition/runtime.rs`, the composition layer's provider-runtime entry point that startup and reload share). You do not need to touch either file to add a provider — editing `models.json` is sufficient.

**How hot reload works (mechanics):**

1. The reload use case (`src/application/catalogue/use_cases/reload_runtime_configuration.rs`, #1849) owns the policy in two phases: a rebuild phase (`rebuild_if_changed` for the pull-based poll, `rebuild` for the forced UDS `reload`) that borrows no runtime and yields a `ReloadStep`, and an `apply` phase that swaps the step into the loop. It drives two ports — a `RuntimeConfigurationSource` and the agent loop as `ReloadRuntime` — and never reads a file or touches a scheduler itself. The UDS dispatch loop (`src/interface/cli/uds_dispatch_reload.rs`) runs the rebuild phase under `tokio::task::spawn_blocking`, so the current-thread UDS runtime keeps accepting, reading and flushing for other clients while a rebuild is in flight (their commands are dispatched in order once it completes), and applies on the dispatch task.
2. The source adapter (`src/infrastructure/runtime_configuration.rs`) watches the configuration the run selected (`--config`, else the global `~/.quecto/config.json` plus the working directory's `.quecto/config.json` overlay and, while an overlay exists, its trust record) and `~/.quecto/models.json` through the reload gate (`src/infrastructure/reload.rs`) by mtime + length + content hash. On a poll, if metadata is unchanged it does **not** read the file (cheap). If metadata changed, it reads and hashes; only a changed hash rebuilds.
3. A rebuild parses the config **once**, composes and publishes the provider runtime through the same injected provider-runtime builder startup used (`ProviderRuntimeBuilder`, threaded from `CliComposition` into the reload inputs), and returns it together with the persisted `tools.policy.entries`; the use case swaps the provider on the loop and re-applies that policy baseline (clearing live-only overlays). The rebuild observes the files before reading them, so a forced `reload` never triggers a second rebuild at the next poll.
4. Polls happen automatically before each prompt, before `set_model`, when `/model` is opened (TUI re-requests the list), and on an explicit UDS `reload`.
5. Reload is **fail-safe**: if the new file is malformed, nothing is published and the last-good provider router stays active — a poll logs a warning and proceeds, an explicit `reload` reports the error — and the session does not crash.
6. Because quecto-tui talks to the agent over UDS, it never needs its own restart either.

So: **edit the file, save, send the next prompt or reopen `/model` — the new provider/model is live.**

## Auto-discover OpenAI-compatible model lists

Discovery is an external sidecar operation: Quecto does not fetch remote catalogs from the registry loader. Instead, run the CLI helper to rewrite `~/.quecto/models.json`; the existing hot reload gate picks up the atomic file replacement.

For any provider in `models.json` with `"api": "openai-completions"` and a `baseUrl` ending at an OpenAI-compatible `/v1` endpoint:

```bash
quecto models discover openrouter
```

The helper calls `GET <baseUrl>/models`, maps each returned entry to the registry model schema (`id` is kept opaque; `name` uses the provider `name`, then `owned_by`, then `id`), and replaces only that provider's `models` array. Other providers, provider options, and `auth` blocks are preserved. `auth.apiKey` may reference `$ENV` or `${ENV}`; discovery resolves it only for the outgoing request and writes the original auth block back unchanged. It never writes `!command` auth.

To keep a catalog fresh, either run a watch loop:

```bash
quecto models discover openrouter --watch --interval 3600
```

or install an external scheduler, for example cron:

```cron
17 * * * * /usr/local/bin/quecto models discover openrouter 2>&1 | logger -t quecto-models-discover
```

A systemd timer should invoke the same one-shot command. Because writes use a same-directory temporary file plus rename, running agents see either the previous complete registry or the next complete registry; malformed JSON is never intentionally published.

## Store a credential (runbook)

Preconditions: `quecto version` works; the agents you will run share the
same `QUECTO_BASE_DIR` (the credential store is `<base_dir>/credentials.json`,
0600, written only by `quecto auth`).

```bash
quecto auth login --provider openai --token sk-proj-…     # → Credential stored for openai
quecto auth login --provider anthropic --token sk-ant-…   # → Credential stored for anthropic
quecto auth login --provider openai --oauth                # browser flow — human at a terminal only; an agent always passes --token
quecto auth login --provider openai --device-code          # headless: prints a URL and a code
quecto auth status                                         # → Credentials:  openai (token) — active
quecto agent --no-session -m "Reply with exactly OK"   # → OK  (proves the default model runs)
quecto auth logout --provider openai                       # rollback → Credential removed for openai
```

`quecto status`'s `OpenAI API:` / `Anthropic API:` lines reflect
`providers.*.api_key` in the global file only, not the credential store —
trust `quecto auth status`. The store takes priority over a config key. An
agent doing this asks the user for the key, never echoes it, never writes
it into a file itself, and never passes `--show-secrets`.

## Where keys go (do not mix these up)

| Provider kind | Where the API key lives | Example |
|---|---|---|
| Built-in `openai-api` / `anthropic-api` | `~/.quecto/config.json` `providers.openai.api_key` / `providers.anthropic.api_key`, or env `OPENAI_API_KEY` / `ANTHROPIC_API_KEY` | `"providers": {"openai": {"api_key": "sk-..."}}` |
| Community / custom API providers | `~/.quecto/models.json` under the provider's `auth.apiKey` | `"auth": {"mode":"apiKey","apiKey":"$FIREWORKS_API_KEY"}` |
| OAuth providers | Kernel credential store via `quecto auth login`; referenced from `models.json` by `auth.oauthProvider` | `"auth": {"mode":"oauth","oauthProvider":"anthropic"}` |

**API key interpolation:** `auth.apiKey` supports `$ENV` and `${ENV}` interpolation, resolved when the registry loads/reloads. Use `$$` for a literal dollar. Prefer env interpolation over committing literal keys.

## Explicit auth modes

Provider keys are auth-specific. Do not overload a single `openai` or `anthropic` key to mean both OAuth and API key — that can silently switch billing mode.

Built-in provider names:

- `openai-api` — OpenAI API key (`OPENAI_API_KEY` or config).
- `openai-oauth` — OpenAI OAuth credential from `quecto auth login --provider openai --oauth`.
- `anthropic-api` — Anthropic API key (`ANTHROPIC_API_KEY` or config).
- `anthropic-oauth` — Anthropic OAuth credential from `quecto auth login --provider anthropic --oauth`.

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
quecto auth login --provider anthropic --token sk-ant-…   # or --oauth at a terminal
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

## Default model and effort per repository (#2024)

A repository pins its own default in its overlay `<repo>/.quecto/config.json`, merged over the global file (`agents.defaults` is merged field-wise, so a repository may pin only the model, only the effort, or both, and inherit the rest). Every agent started in that directory — `quecto agent`, a `quecto-tui` tab, a spawned child — starts on it; a sibling repository without an overlay starts on the global default. Three ways to pin one, all through the same safe writer (only the addressed key changes, the result is validated before a byte is written, trust is recorded for the overlay):

| From | Repository default | Global default |
|---|---|---|
| CLI | `quecto config set agents.defaults.model '"openai-api/gpt-5.6-luna"'` (and `agents.defaults.effort '"high"'`) | add `--global` |
| UDS | `{"type":"set_model","model":"openai-api/gpt-5.6-luna","persist":"local"}` / `{"type":"set_effort","effort":"high","persist":"local"}` | `"persist":"global"` |
| quecto-tui | `/model`, Tab until the footer reads *use and pin as this repo's default*, Enter | Tab once more: *use and pin as the global default* |

Verify with `quecto config get --effective agents.defaults.model` and `quecto status` (`Overlay:` must read `(trusted)`); a new agent's `get_state` reports the pinned `model` and `effort`. Roll back with `quecto config unset agents.defaults.model` (add `--global` for the global file); setting the key to `null` would *override* the global value with null, which is why removal is its own command. A pin over UDS or from the TUI is refused — with the session left unchanged — when the overlay is not trusted (`quecto config trust` first), when the overlay location is a symbolic link, when the run was started with an explicit `--config` (no overlay applies), or for a bare model id (only `provider/model` is recorded). `quecto-tui --model <m> --effort <e>` and `quecto agent --model <m> --effort <e>` choose for one run without writing anything.

## Internal agent guidance

1. Prefer `models.json` for community/runtime additions.
2. Use explicit auth-specific provider keys (`*-api`, `*-oauth`).
3. OAuth references must be one of the kernel-known identities (`openai`, `anthropic`); otherwise the provider must be an API-key/sidecar provider.
4. Changes are hot-loaded on consume; no restart is required.
5. When in doubt about the schema, read the `docs` tool's `models` page (`docs {"name": "models"}`) rather than guessing; this file is the full human reference.

## Retry-After hints and the wait budget

A provider `retry-after` (seconds) or `retry-after-ms` hint is honoured up to
the session's retry wait budget (30 seconds by default). A hint above that
budget is not clamped: the request fails immediately with the provider error so
the caller can decide, instead of silently waiting longer than the budget. This
applies to every session, not only swarm runs.

## Stalled replies

No provider request has a total time limit while it streams: a long reply that
keeps sending is never cut short. A streaming reply that sends *nothing* — no
response head, no bytes, no SSE event or keep-alive — for 300 seconds is
abandoned (the stream idle limit; the official Codex client allows the same,
since a reasoning model can think silently for minutes). A non-streaming reply
sends nothing until it is complete, so it has a 20 minute total limit instead.
Either expiry fails the attempt with a `stream idle timeout: …` or
`reply timeout: …` error of class `stalled`, recorded on the attempt as `Idle`
or `TimedOut`. A stall is retried at most once per request (before any output
reached the caller); an error status whose body stalls keeps its status class
and shows `(error body abandoned: …)` in place of the body. Neither limit is
configurable.

## Runaway replies

A reply that keeps sending is never cut short by time, so a runaway — a model
stuck in a repetition loop, on a backend that takes no output limit (the Codex
ChatGPT backend refuses `max_output_tokens`) — is bounded by size instead. Each
attempt may stream 8 bytes of output per token of the model's declared output
limit (32,768 tokens when the model's limit is not known, and never less than
the request's own `max_tokens`): 1,024,000 bytes for a 128k-token model. A
token is about four bytes, so a reply within its limit stays well under the cap.
Output means the text, thinking, refusal and tool-call argument deltas, not
the events around them.

An attempt past its cap is abandoned with an `output cap exceeded: …` error of
class `output_capped`, recorded on the attempt as `OutputCapped`. It is **not
retried**: a runaway tends to repeat, each time at the cost of the whole cap,
and by then the reply has usually streamed output a retry could not replay. The
turn fails with guidance to narrow or rephrase the request. The cap covers
every streaming path of a request the agent loop sends (incremental and
assembled; Codex, OpenAI-compatible and Anthropic; gated or not); a whole
non-streaming JSON reply is bounded by the 20 minute reply limit instead. The
cap is not configurable.

## Replies cut short

A streamed reply is whole only when its protocol says it ended: an
`anthropic-messages` reply at its `message_stop` event, an OpenAI Responses
reply at `response.completed` (or `data: [DONE]`), and an
`openai-completions` reply at `data: [DONE]` or a chunk naming its
`finish_reason`. A body that ends before that — output or not — is a reply cut
short: the attempt fails with `… ended without completion: connection closed
before …` (class `network`, retried before any output reached the caller) and
is recorded as `CutShort`, on every path (streamed or read whole, gated or
not); it is never taken as a whole answer. A 200 body with no event at all is
an empty stream (class `empty_stream`, retried). Tokens a cut-short reply
already reported (Anthropic's `message_start` usage, an OpenAI usage chunk, a
Responses event's `usage`) are still counted, on a failed request and on one a
retry completed.

So an **Anthropic-compatible endpoint** (an `anthropic-messages` provider with
a custom `baseUrl`) must send `message_stop` at the end of every reply, as the
Anthropic Messages API specifies; one that closes the stream without it fails
every request. An OpenAI-compatible endpoint must send `data: [DONE]` or a
`finish_reason`.

## Progress and interrupted requests

While a request is in flight, `get_state` reports it as `modelTurn`: how long
it has run and, for its attempt in flight, the events and output bytes it has
streamed and how long since its last event (see `get_state` in
[uds-protocol.md](uds-protocol.md)). A turn that stops while a request is in
flight — a one-shot run's `--max-time` deadline, or, for a UDS agent (every
subagent), an `abort` or `steer` or a shutdown it handles (SIGTERM/SIGINT, a
`shutdown` command, its parent or last client going away) — writes that request to the event log as a `request_observed`
record with outcome `cancelled`; its attempt in flight appears in
`attempt_diagnostics` with termination `Interrupted`, its `event_count`,
`output_bytes` and `first_token_ms` as they stood. A SIGKILL cannot be handled
and leaves no record.

## Where a request's input changed

A Responses (`codex`) request of a named session also says, in its
`request_observed` record, how its input relates to the session's last
accepted request. That tells a cache miss caused by the harness changing an
earlier input item from one the provider caused. It records counts, indices,
a kind and token estimates, never content:

- `input_items`: the input items the request sent;
- `previous_items`: the input items of the request it was compared with;
  `null` when there was none to compare with (the session's first request
  in this process);
- `first_changed_item`: the first item whose serialized bytes differ from the
  previous request's item at the same index (or, when the input got shorter,
  the first item it lacks); `null` when the previous input is a byte-identical
  prefix of this one (append-only), or when nothing was compared;
- `first_changed_kind`: that item's kind (`user`, `assistant`,
  `function_call`, `function_call_output` or `reasoning`), `null` when there
  is none;
- `prefix_tokens_estimate`: the estimated tokens of the unchanged input items;
- `unchanged_prefix_tokens_estimate`: the estimated tokens of the unchanged
  prefix of the whole request: the instructions and tools, when they, the
  model and the endpoint are unchanged, then the unchanged input items; `0`
  when any of those changed, since the service caches per model;
- `request_tokens_estimate`: the estimated tokens of the whole request
  (instructions, tools and every input item).

The three estimates use one estimator, so compare
`cache_read_tokens / input_tokens` with
`unchanged_prefix_tokens_estimate / request_tokens_estimate`. Before reading
them, allow for what the service does and the estimator does not know:

- the service caches only a prompt of 1,024 tokens or more, and caches in
  128-token blocks, so expect `cache_read_tokens` at most the unchanged prefix
  rounded down to a multiple of 128, and `0` for a request below 1,024;
- encrypted reasoning items are opaque: they are estimated at the plain
  ASCII rate of 4 characters a token, not as the reasoning the service counts,
  so a reasoning-heavy prefix can be off in either direction.

So, with `previous_items` set and `first_changed_item` null, a cached share far
below the expected one (beyond those allowances) means the provider missed
its cache. With `first_changed_item` set, the harness changed that item, and
nothing after it can be cached.

The baseline is the session's: its agent loop keeps it and hands it to every
request, so a provider rebuilt mid-session (an OAuth refresh) keeps
comparing. It holds one 64-bit digest per item and a digest of the session
key, never the items or the key, and a request replaces it only once the
provider accepts a send of it (a reply, or a streamed event that is not an
error): a refused, failed or cancelled send never becomes it, and when
replayed reasoning is refused the request resent without it does. A retry
records what its first send found. A request without a session is neither
compared nor kept. Other providers do not record these fields.
