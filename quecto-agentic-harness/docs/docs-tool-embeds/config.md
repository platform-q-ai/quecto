# Configuration: files, precedence, and how to change a setting

Which files this run loaded, how they combine, and the commands that change
them safely. Use the commands; do not hand-edit JSON unless a command cannot
express the change.

## Files and precedence

- `--config <path>` — an explicit selection replaces everything below; the file must exist.
- `<base_dir>/config.json` — the **global** file (`~/.quecto/config.json`, or `$QUECTO_BASE_DIR/config.json`). Absent means defaults. Holds `providers` (API keys, endpoints) and `admission`; those two sections are global-only.
- `./.quecto/config.json` — the **repo-local overlay** in the process working directory (never a parent directory), merged over the global file: `agents.defaults` field-wise, `tools.web` field-wise per engine, `tools.policy.entries` entry-wise, `container_configs` entry-wise (a local `"default": true` un-defaults the global entries), `workflow` field-wise. An overlay carrying `providers` or `admission` is refused with an error naming the key.
- The overlay is applied only when **trusted**: its exact content (canonical path + sha256, one content per path) must be recorded in `<base_dir>/config-overlay-trust.json`. An untrusted overlay is reported on stderr and not applied; when `quecto config trust` would refuse it (not JSON, not an object, a global-only section, an invalid value) the report says so and why, instead of sending you to a command that will turn you away. `quecto config set` records trust for what it writes; a hand edit revokes trust until `quecto config trust` is run again (reverting the edit does not restore it). The overlay must be a **regular file in a regular `.quecto` directory**: a symbolic link at `./.quecto/config.json` or at `./.quecto` is refused whatever it points at (`status` shows `(refused)`, `config trust` and `config set --local` exit 1, and nothing is written through the link), because trust is keyed by the file's identity and a link would borrow its target's.
- A `./config.json` directly in the working directory is a retired mechanism: it is not loaded, and `quecto status` warns while one that looks like a quecto configuration exists. Move its repo-specific keys into the overlay with `quecto config set` and any `providers`/`admission` section into the global file.

`QUECTO_*` environment variables (for example `QUECTO_AGENTS_DEFAULTS_MODEL`) override the merged files for that process and are never written back.

## Runbook: change a setting

To set a value **for this repository** (writes `./.quecto/config.json`, creating it if needed, and marks it trusted):

```
quecto config set agents.defaults.model '"openai-api/gpt-5.5"'
quecto config set agents.defaults.effort '"high"'
quecto config set tools.policy.entries.native:bash '{"scope":"parent"}'
```

To make the same change **global** add `--global` (writes `<base_dir>/config.json`):

```
quecto config set --global agents.defaults.model '"openai-api/gpt-5.5"'
quecto config set --global providers.openai.api_base '"https://api.example"'
```

Values are JSON. A bare word that is not valid JSON is taken as a string (`quecto config set agents.defaults.model gpt-5.5`). Paths are dotted keys; a segment that is not an object on the way is an error.

**Verify** what a run in this directory will use:

```
quecto config get --effective                      # the whole merged document
quecto config get --effective agents.defaults.model
quecto config get --global agents.defaults         # one layer, as written
quecto config get --local                          # the overlay, as written (trusted or not)
quecto status                                      # Config:, Overlay: (trusted|untrusted|refused|none), Model:, Effort:
```

Secret-shaped values — keys named `api_key`, `apiKey`, `token`, `secret`, `password`, or ending in `_key` / `_token` — print as `"<redacted>"` and stderr says how many were hidden. This output lands in your context and in transcripts, so leave it that way. To check whether an API key is set, use `quecto status` (`OpenAI API:    configured` / `not set`): an explicit empty `"api_key": ""` also prints as `"<redacted>"`, so a redacted read-out does not mean the key is usable. Only a human at a terminal should pass `--show-secrets`.

**Trust** an overlay you did not write (review it first; the command refuses one that carries a global-only section or is not a valid configuration):

```
quecto config trust                 # ./.quecto/config.json
quecto config trust --path <file>
```

**Unset** a key you (or someone) set, in the layer it is set in (a key the layer does not set is an error naming the layer, so a rollback aimed at the wrong file says so):

```
quecto config unset agents.defaults.model            # ./.quecto/config.json
quecto config unset --global agents.defaults.effort  # <base_dir>/config.json
```

Setting a key to `null` is not the same: in the overlay it *overrides* the global value with null. Emptied parent objects are kept.

**Rollback**: `quecto config unset` the key, set it back, or remove the overlay file (`rm ./.quecto/config.json`) to return to the global file alone. Only the addressed key's value changes in a patched file: the writer keeps every other key and their order, validates the result before writing — the file on its own *and* the configuration this directory would then load (global merged with the trusted overlay), so an overlay change that would leave, say, no default container config is refused before it can break every later run — and replaces the file atomically (tmp + fsync + rename). It normalises layout: pretty-printed JSON in the file's own indentation (two spaces for a new or compact file), LF line endings, one trailing newline; a file already in that layout changes only on the touched line. A refused value (for example an unknown effort level) leaves the file byte-identical and exits 1 with the reason. Concurrent `config set`s of one file serialise on a lock under `<base_dir>/locks/` (never beside the file: a refused `config set --local` in a clean checkout creates nothing there, not even `.quecto/`), so none loses another's key. `.quecto/` holds only `config.json`; there is nothing else to ignore.

## Runbook: pin a default model (and effort) for this repository

Preconditions: you are in the repository's root (the overlay is discovered in the working directory only), the model is a qualified `provider/model` id the catalogue can run (`list_models` over UDS lists it as `configured`), and any existing `./.quecto/config.json` is trusted (`quecto status` shows `Overlay: … (trusted)`; if `(untrusted)`, review it, then `quecto config trust`).

```
quecto config set agents.defaults.model '"openai-api/gpt-5.6-luna"'
quecto config set agents.defaults.effort '"high"'     # optional; must be a level the model accepts
```

Verify:

```
quecto config get --effective agents.defaults.model   # "openai-api/gpt-5.6-luna"
quecto status                                         # Overlay: … (trusted)  Model: openai-api/gpt-5.6-luna  Effort: high
```

Every agent started in this directory from now on — `quecto agent`, a `quecto-tui` tab, a spawned local child — starts on that model (its `get_state` reports it); other repositories keep the global default. The global file is untouched (`quecto config get --global agents.defaults.model` still prints the old value). Your own running session is not switched: send `set_model` / `set_effort` for it, or leave it. From a running session the same record can be made with `set_model … "persist":"local"` (`"global"` for the global file) — it goes through this writer and is refused with the same reasons; the TUI's `/model` selector offers it as *use and pin as this repo's default* (Tab). To pin the same model for every repository use `--global` instead.

Rollback:

```
quecto config unset agents.defaults.model             # the global default applies again
quecto config unset agents.defaults.effort
```

`quecto config get --effective agents.defaults.model` then prints the global value. `rm ./.quecto/config.json` removes every repository setting at once.

## Rules that matter mid-task

- Configuration is read at startup and on reload; a running agent re-reads the base file and the overlay (and, while an overlay exists, the trust record) before the next turn, `set_model`, or a forced `reload`. What a reload carries into the running loop is the **providers** and the **tool policy**: a `config set` of `tools.policy.entries…` or of a provider endpoint from a task takes effect for the next turn of this run. `agents.defaults.model` and `agents.defaults.effort` written now do **not** change the running loop's model or effort — use `set_model` / `set_effort` for this run; the file value applies to the next run and to every subagent spawned locally afterwards. Removing the overlay follows the same rule. Do not create or edit configuration unless the task is to change Quecto's configuration, and say so in your report when you do.
- Local subagents inherit your working directory and perform their own discovery there (same global file, same overlay when trusted); a parent's explicit `--config` is not forwarded to them (pass `config` in the spawn call, or set `QUECTO_RUNTIME_CONFIG_PATH`, to pin one). Container subagents are handed the parent's base file; the overlay is not forwarded into containers.
- `set_tool_policy … persist` over UDS writes `tools.policy.entries` of the base file through the same writer; it never touches the overlay, and an entry the overlay already defines is refused with the `quecto config set` command to run instead.
- **Container spawns do not read the overlay yet (#2024 S4).** `spawn` with `container: true` loads `container_configs` from the base file plus the separate repo-local container mechanism (its own trust record and prompt; `quecto config trust` does not cover it). `container_configs` in an overlay are visible to `config get --effective` and `status` today, not to `spawn`. See `docs {"name": "subagents"}`.
