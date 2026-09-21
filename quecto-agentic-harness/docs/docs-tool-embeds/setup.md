# Setting up quecto (start here)

Pick the row that matches the task, read the one page it names, run the
goal command, run the verify command and compare with the expected output,
report what you changed. Every command is a `bash` call from the
**repository root** (the process working directory); `quecto` is on PATH
(`quecto version` prints `quecto <x.y.z>`). Nothing here needs a hand-edited
JSON file: every write goes through a `quecto` command that validates the
result and prints the file it touched (`auth login` prints `Credential stored
for <provider>` instead of a path).

In the TUI, `/setup` submits this walkthrough for you (`/setup model <id>`,
`/setup admission`, `/setup podman`, `/setup auth` for one area).

## Decision tree

| Situation | Page | Goal command | Verify (expected) | Rollback |
|---|---|---|---|---|
| First install: no credential yet | `docs {"name": "models"}` | `quecto auth login --provider openai --token <key>` (or `--provider anthropic`; always `--token` from an agent — without it the browser OAuth flow blocks the call) | `quecto auth status` → `openai (token) — active` | `quecto auth logout --provider openai` |
| New repo: a setting for this repo only, global untouched | `docs {"name": "config"}` | `quecto config set <dotted.key> <json>` | `quecto config get --effective <dotted.key>` prints the value; `quecto status` → `Overlay: <repo>/.quecto/config.json (trusted)` | `quecto config unset <dotted.key>`; `rm .quecto/config.json` drops the whole overlay |
| `quecto status` says `Overlay: … (untrusted)` (a hand-written or pulled overlay) | `docs {"name": "config"}` | review it: `quecto config get --local`, then `quecto config trust` | `quecto status` → `Overlay: <repo>/.quecto/config.json (trusted)` | `rm .quecto/config.json` |
| Pin the default model (and effort) for this repo | `docs {"name": "models"}` | `quecto config set agents.defaults.model '"<provider/model>"'` | `quecto config get --effective agents.defaults.model` → `"<provider/model>"`; optionally one model call: `quecto agent --no-session -m "Reply with exactly OK"` prints `OK` | `quecto config unset agents.defaults.model` |
| Enable the admission broker (one per host) | `docs {"name": "admission-broker"}` | `quecto config set --global admission '{"groups":…,"aliases":…,"bindings":…}'` (the exact JSON is on that page; `{}` is refused) then `quecto admission-broker install-service` (`--dry-run` first) | `quecto admission-broker status` → `{"directory":…,"epoch":1,"journal_healthy":true,…}` | `quecto config unset --global admission`, restart agents, then `quecto admission-broker uninstall-service --directory <base_dir>/admission` |
| A podman/docker container for this app | `docs {"name": "container-runtime"}` | `quecto container init` (its `standard` entry becomes this repo's default: `container: true` selects it, no global default overrides it), then, if `.quecto/containers/standard/Containerfile` is still the starter, give it this repo's toolchain and `ai.quecto.required-tools` label (runbook step 2: read manifests + CI, show, ask; it is the repo's own: commit it), then the `podman build …` line init printed | `quecto container status` → last line `ready: spawn {"container":true} …`; `quecto container doctor` → no `✗` line, exit 0 (`! gh` is a warning) | `quecto config unset --local container_configs.standard`; `rm -r .quecto/containers/standard` |
| A swarm (many agents, one container) | `docs {"name": "swarm"}` | the container row first, then `spawn … "container":{"mode":"new","container_config":"standard"}` | the spawn result names `container_config=standard`; `agent_cmd get_containers` lists it `running` | `agent_cmd kill_container` or `quecto container kill <ref\|name>` |

A `quecto` command that changes a file prints the path it wrote and exits 0;
a refusal prints the reason and exits 1 with the file byte-identical (`auth
logout` of a missing credential is the exception: `no credential found`, exit 0). Treat
exit 1 from a *write* as "not done", never retry the same command unchanged
(a `config get` of an unset key also exits 1, with `is not set` — that is an answer).

## Order on a fresh machine

1. Credential — `quecto auth login …`, then `quecto auth status`.
2. Global default model (optional) — `quecto config set --global agents.defaults.model '"<provider/model>"'`.
3. Repo overlay — `cd <repo>` and `quecto config set …` for what this repo needs.
4. Admission — only when the user wants provider requests bounded across sessions.
5. Container — only for repos whose subagents should run isolated.
6. `quecto status` last: every line must match what you set.

## Where things live

| Path | What | Written by |
|---|---|---|
| `<base_dir>/credentials.json` | API keys and OAuth tokens | `quecto auth login` only |
| `<base_dir>/config.json` | the global file: `providers`, `admission`, global defaults | `quecto config set --global …` |
| `<repo>/.quecto/config.json` | the repo overlay: `agents.defaults`, `tools`, `workflow`, `container_configs` | `quecto config set …`, `quecto container init` |
| `<base_dir>/config-overlay-trust.json` | which overlay content is approved | `quecto config set`, `quecto config trust`, `quecto container init` |
| `<base_dir>/models.json` | extra providers/models | you, with the schema on the `models` page; `quecto models discover` |
| `<base_dir>/admission/` | the broker's sockets, lock and journal | `quecto admission-broker run` / the service |
| `<repo>/.quecto/containers/standard/` | Containerfile and runtime scripts | `quecto container init` |

`<base_dir>` is `~/.quecto` unless `QUECTO_BASE_DIR` is set; `quecto status`
prints the two config paths on its `Config:` and `Overlay:` lines.

## Rules for an agent doing this

- Read before you write: `quecto status`, then `quecto config get --effective`.
  Secrets print as `"<redacted>"`; never pass `--show-secrets`, never `cat`
  `credentials.json`, never echo a key the user pasted.
- Prefer the repo overlay; write the global file (`--global`) only when the
  task says "for every repo" or the key is global-only (`providers`, `admission`).
- One key per `config set`; run the verify command after each; say which
  file changed in your report.
- Your own running session keeps its model and admission state until it is
  restarted: a pinned model applies to the next run, an enabled broker to
  agents started after it. Say so instead of claiming the current session changed.
- Do not run `quecto admission-broker run` from a tool call (it stays in the
  foreground and dies with the tool); use `install-service`.
