# Configuration: files, precedence, and how to change a setting

Which files this run loaded, how they combine, and the commands that change
them safely. Use the commands; do not hand-edit JSON unless a command cannot
express the change.

## Files and precedence

- `--config <path>` — an explicit selection replaces everything below; the file must exist.
- `<base_dir>/config.json` — the **global** file (`~/.quecto/config.json`, or `$QUECTO_BASE_DIR/config.json`). Absent means defaults. Holds `providers` (API keys, endpoints) and `admission`; those two sections are global-only.
- `./.quecto/config.json` — the **repo-local overlay** in the process working directory (never a parent directory), merged over the global file: `agents.defaults` field-wise, `tools.web` field-wise per engine, `tools.policy.entries` entry-wise, `container_configs` entry-wise (a local `"default": true` un-defaults the global entries), `workflow` field-wise. An overlay carrying `providers` or `admission` is refused with an error naming the key.
- The overlay is applied only when **trusted**: its exact content (canonical path + sha256) must be recorded in `<base_dir>/config-overlay-trust.json`. An untrusted overlay is reported on stderr and not applied. `quecto config set` records trust for what it writes; a hand edit revokes trust until `quecto config trust` is run again.
- A `./config.json` directly in the working directory is a retired mechanism: it is not loaded, and `quecto status` warns while it exists. Move its repo-specific keys into the overlay with `quecto config set` and any `providers`/`admission` section into the global file.

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
quecto status                                      # Config:, Overlay: (trusted|untrusted|none), Model:, Effort:
```

**Trust** an overlay you did not write (review it first; the command refuses one that carries a global-only section or is not a valid configuration):

```
quecto config trust                 # ./.quecto/config.json
quecto config trust --path <file>
```

**Rollback**: set the key back, or remove the overlay file (`rm ./.quecto/config.json`) to return to the global file alone. Nothing else in a patched file changes: the writer keeps unknown keys, key order and the file's indentation, validates the result as a configuration before writing, and replaces the file atomically (tmp + fsync + rename). A refused value (for example an unknown effort level) leaves the file byte-identical and exits 1 with the reason.

## Rules that matter mid-task

- Configuration is read at startup and on reload; a running agent re-reads both files before the next turn, `set_model`, or a forced `reload`. Writing the overlay from a task takes effect for the next turn of this run and for every subagent spawned locally afterwards. Do not create or edit configuration unless the task is to change Quecto's configuration, and say so in your report when you do.
- Local subagents inherit your working directory and perform their own discovery there (same global file, same overlay when trusted); a parent's explicit `--config` is not forwarded to them (pass `config` in the spawn call, or set `QUECTO_RUNTIME_CONFIG_PATH`, to pin one). Container subagents are handed the parent's base file; the overlay is not forwarded into containers.
- `set_tool_policy … persist` over UDS writes `tools.policy.entries` of the base file through the same writer; it does not touch the overlay.
- The overlay's `container_configs` entries are also what `spawn` with `container: true` resolves once the overlay is trusted. See `docs {"name": "subagents"}`.
