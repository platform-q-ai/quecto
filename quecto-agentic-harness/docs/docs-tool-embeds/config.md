# Configuration (runbook): files, precedence, and how to change a setting

Two files, one merge, one writer. Use the commands; hand-edit JSON only
when no command can express the change (and then `quecto config trust`).

## Files and precedence

- `--config <path>` — an explicit file replaces everything below; it must exist.
- `<base_dir>/config.json` — the **global** file (`~/.quecto/config.json`, or `$QUECTO_BASE_DIR/config.json`; absent = defaults). `providers` (keys, endpoints) and `admission` live here and **only** here (global-only).
- `./.quecto/config.json` — the **repo overlay** in the process working directory (never a parent), merged over the global file: `agents.defaults` field-wise, `tools.web` field-wise per engine, `tools.policy.entries` entry-wise, `container_configs` entry-wise (an overlay `"default": true` un-defaults the global entries), `workflow` field-wise; anything else replaced whole. An overlay carrying `providers` or `admission` is refused naming the key.
- The overlay applies only when **trusted**: its canonical path + sha256 is recorded in `<base_dir>/config-overlay-trust.json`. `quecto config set` records trust for what it writes; a hand edit revokes it until `quecto config trust` (reverting the edit does not restore it). A symbolic link at `./.quecto` or `./.quecto/config.json` is refused whatever it points at.
- A `./config.json` directly in the working directory is retired: not loaded; `quecto status` warns while one that contains quecto keys (`agents`, `tools`, …) exists. Move its keys with `quecto config set`.
- `QUECTO_*` environment variables (`QUECTO_AGENTS_DEFAULTS_MODEL`, …) override the merged files for an **agent run** only; `config get --effective` and `status` show file values and never reflect them, and they are never written back.

## Preconditions

- You are in the repository root (`pwd` = the directory the agent was started in): the overlay is discovered there only.
- `quecto status` exits 0. Read its `Overlay:` line: `none` (no overlay yet), `(trusted)`, `(untrusted)` or `(refused)`. With `(untrusted)` review the file (`quecto config get --local`) and run `quecto config trust` before any `config set`, which refuses to patch an untrusted overlay.
- Values are JSON: quote strings (`'"openai-api/gpt-5.5"'`); a bare word that is not JSON is taken as a string; numbers, `true`, objects as written.

## Do

For **this repository** (writes `./.quecto/config.json`, creates it, records trust):

```
quecto config set agents.defaults.model '"openai-api/gpt-5.6-luna"'
quecto config set agents.defaults.effort '"high"'     # only for a model with effort levels (docs models)
quecto config set tools.policy.entries.native:bash '{"scope":"parent"}'
```

Expected: `set agents.defaults.model in /repo/.quecto/config.json (created) (trusted)`, exit 0 (`(created)` only when the file did not exist). The writer validates the *file*, not the model id against the catalogue nor the effort against the model: prove the pair runs (`docs {"name": "models"}`, Verify).

For **every repository** (writes `<base_dir>/config.json`):

```
quecto config set --global agents.defaults.model '"openai-api/gpt-5.6-luna"'
quecto config set --global providers.openai.api_base '"https://api.example"'
```

Expected: `set agents.defaults.model in /home/me/.quecto/config.json` (`(created)` when the global file did not exist), exit 0.

Approve an overlay someone else wrote (review it first):

```
quecto config get --local            # print it, as written
quecto config trust                  # → trusted /repo/.quecto/config.json (sha256 …)
```

## Verify

```
quecto status                                         # first: the trust line
quecto config get --effective agents.defaults.model   # → "openai-api/gpt-5.6-luna"
quecto config get --effective                         # the whole merged document
quecto config get --global agents.defaults            # one layer, as written
quecto config get --local                             # the overlay, as written (trusted or not)
```

A `get` of a key the addressed layer does not set prints `` `x.y` is not set `` and exits 1 — a fact, not a failure (an untrusted overlay's warning is printed only when the key resolves; the trust line is on `status`).

Expected `status`:

```
quecto Status
  Config:    /home/me/.quecto/config.json
  Overlay:   /repo/.quecto/config.json (trusted)
  Workspace: /home/me/.quecto/workspace
  Model:     openai-api/gpt-5.6-luna
  Effort:    high
  OpenAI API:    configured
  Anthropic API: not set
```

`Workspace:` is `agents.defaults.workspace` (not moved by `QUECTO_BASE_DIR`).
`OpenAI API:` / `Anthropic API:` reflect `providers.*.api_key` in the global
file only; a credential stored by `quecto auth login` shows in `quecto auth
status`, not here. Secret-shaped keys (`api_key`, `token`, `secret`,
`password`, `*_key`, `*_token`) print as `"<redacted>"` — leave it that way;
only a human at a terminal passes `--show-secrets`.

## Rollback

```
quecto config unset agents.defaults.model            # → unset agents.defaults.model in /repo/.quecto/config.json (trusted)
quecto config unset --global agents.defaults.effort  # the global file
rm ./.quecto/config.json                             # drop every repo setting at once
```

`unset` removes the key in the layer named (default: overlay); a key that layer
does not set is an error naming the layer, exit 1, nothing written. Setting a
key to `null` is not a rollback: in the overlay it *overrides* the global value
with null. Emptied parent objects are kept.

## If it fails

| Symptom | Cause | Fix |
|---|---|---|
| ``overlay … is not trusted (sha256 …); review it and run `quecto config trust` first``, exit 1 | the overlay was hand-written or edited after the last `config set` | `quecto config get --local`, review, `quecto config trust`, retry |
| ``refusing to trust …: `providers` is global-only …`` | the overlay carries `providers` or `admission` | `quecto config set --global <key> <value>` for each of its keys, then hand-edit the overlay to delete the section (the one edit no command can make: an untrusted overlay is never patched), then `quecto config trust` |
| `refusing to write …: the result is not a valid configuration: …` (e.g. `invalid effort level 'x'; expected one of: none, low, medium, high, xhigh, max`) | the value is rejected by validation of the file or of the merge | use a valid value; the file is byte-identical |
| `Overlay: … (refused)` | `.quecto` or `.quecto/config.json` is a symbolic link | replace the link with a regular directory/file, then `quecto config trust` |
| ``…: `x.y` is not set in …; nothing to unset (the other layer may set it …)`` | rollback aimed at the wrong layer | `quecto config get --global x.y` / `--local`, then `unset` with the right flag |
| `status` warns about `./config.json` | a pre-#2024 replace-style file | `quecto config set` its keys into the overlay (global-only ones with `--global`), delete it |
| a value you set does not show in `--effective` | you are in another directory, or the overlay is untrusted | `pwd`; `quecto status` |
| an agent runs on a value `--effective` does not show | a `QUECTO_*` env override (invisible to `get`/`status`), or an explicit `--config` run (which ignores both layers) | `env \| grep QUECTO_`; run without `--config` |
| `config get --effective agents.defaults.model` is right but this session still answers on the old model | file values apply to the next run | `set_model` / `set_effort` for the current session, or restart it |

The writer patches only the addressed key (other keys and their order kept),
validates the file alone *and* the merge this directory would load, writes
atomically (tmp + fsync + rename, pretty JSON in the file's own indentation,
LF, one trailing newline) and serialises on a lock under `<base_dir>/locks/`,
so concurrent sets never lose each other's keys and a refused set in a clean
checkout creates nothing — not even `.quecto/`.

## Rules that matter mid-task

- A running agent re-reads both files (and the trust record) before its next turn, `set_model`, or a forced `reload`; what a reload carries into the running loop is the **providers** and the **tool policy**. `agents.defaults.model`/`effort` written now apply to the next run and to every subagent spawned locally afterwards — use `set_model`/`set_effort` for this run (`"persist":"local"` does both). Do not create or edit configuration unless the task is to change quecto's configuration, and say so in your report when you do.
- Local subagents inherit your working directory and do their own discovery there (same global file, same overlay when trusted); a parent's `--config` is not forwarded (pass `config` in the spawn call, or `QUECTO_RUNTIME_CONFIG_PATH`). Container subagents get the parent's base file; the overlay is not forwarded into containers.
- `set_tool_policy … persist` over UDS writes `tools.policy.entries` of the base file through the same writer; an entry the overlay already defines is refused with the `quecto config set` command to run instead.
- **Container spawns read the effective configuration** fresh at every spawn: `spawn` with `container: true` (or a named `container_config`) selects from the merged `container_configs`; an untrusted or refused overlay contributes nothing (the tool result carries the diagnostic; `quecto config trust` is the one approval). Binding a repository to a container: `docs {"name": "container-runtime"}` (`quecto container init` writes the entry for you; a hand-rolled entry is `quecto config set --local container_configs.<name> '{"default":true,"create":[…],"exec":[…],"inspect":[…],"kill":[…],"cleanup":[…]}'` with absolute script paths, undone with `quecto config unset --local container_configs.<name>`; a merge that would leave no default, or two, is refused).
