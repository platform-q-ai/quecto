# Configuration file selection (deep dive)

Which `config.json` this run loaded, and what that means for files you write.

## Precedence

Quecto loads exactly one configuration file per process, chosen in this order:

1. `--config <path>` — explicit; the file must exist.
2. `./config.json` in the process working directory — the directory itself only, parents are never searched.
3. `<base_dir>/config.json` — the global file (`~/.quecto/config.json`, or `$QUECTO_BASE_DIR/config.json`); absent means defaults apply.

Files are selected, never merged: a working-directory `config.json` fully replaces the global one. Only the *absence* of `./config.json` falls through — a present file that is invalid JSON, a directory, a dangling symlink, or unreadable is a startup error naming the path. `quecto status` prints the selected file as `Config:`.

## Rules that matter mid-task

- Your workspace is the process working directory. A `config.json` you write there is **not** read by this run (config is selected at startup) but **is** loaded by every subagent spawned locally afterwards, and by the next run launched from that directory. Do not create or edit `./config.json` unless the task is to change Quecto's configuration, and say so in your report when you do.
- Provider settings, `tools.*`, `container_configs`, workflow templates and agent defaults all come from the selected file. To change a global default (for example `agents.defaults.model`), edit the file `quecto status` names — which may be a project-local file, not `~/.quecto/config.json`.
- Local subagents inherit your working directory and perform their own discovery there; a parent's explicit `--config` is not forwarded to them (pass `config` in the spawn call, or set `QUECTO_RUNTIME_CONFIG_PATH`, to pin one). Container subagents are always handed the parent's selected file.
- The repo-local `.quecto/config.json` is a different, narrower mechanism: it contributes container definitions only and is trust-gated by content hash. See `docs {"name": "subagents"}`.
