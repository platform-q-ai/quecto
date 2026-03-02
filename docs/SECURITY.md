# Quecto Security Model

This document describes the actual runtime isolation guarantees in quecto.
It was written to close issue #201, which audited isolation gaps following
the removal of the WASM sandbox in #194.

---

## Directory layout and trust boundary

```
~/.quecto/                    ← base_dir  (trusted, not agent-accessible)
├── config.json               ← provider API keys, channel tokens
├── credentials.json          ← live OAuth tokens (mode 0600)
├── sessions/                 ← conversation history
├── cron.json                 ← scheduled jobs
└── workspace/                ← sandbox root  (agent read/write boundary)
```

`credentials.json` and `config.json` live one level **above** the workspace.
`Sandbox::validate_path()` resolves symlinks and enforces that all filesystem
tool calls stay inside `workspace/`. An agent cannot read credentials via the
`read`, `write`, `edit`, or `ls` tools.

---

## Tool isolation by layer

### bash (exec)

**Isolation: nsjail (Linux namespaces + rlimits) — strongest boundary.**

- Fresh network, PID, mount, and IPC namespaces per invocation
- Workspace mounted read-write; `/bin`, `/usr`, `/lib` read-only; no `/home`, no `~/.quecto`
- Memory capped via `--rlimit_as` (default 4 GB virtual), PID limit, CPU and wall-clock timeouts
- Environment: `env_clear()` + strict allowlist (`HOME`, `PATH`, `LANG`, `TZ`, `TERM`,
  `SHELL`, `USER`, `LOGNAME`, `TMPDIR`, `LC_*`)
- All `QUECTO_*` vars stripped at **two independent points**: `build_source_env()` filters
  by `SECRET_ENV_PREFIX = "QUECTO_"` before the env map is built; `apply_nsjail_env()`
  applies a second `QUECTO_*` check before passing vars to the subprocess
- Network disabled by default (`network_passthrough: false`); opt-in per config

**Result:** A bash command cannot read credentials from disk (not mounted), cannot
see `QUECTO_*` env vars, and cannot make outbound network calls unless the operator
explicitly enables `network_passthrough`.

### read / write / edit / ls / grep / find (filesystem)

**Isolation: `Sandbox::validate_path()` — workspace boundary only.**

- Resolves symlinks via `canonicalize()` before comparing against workspace root
- Symlinks pointing outside the workspace are blocked (tested)
- `~/.quecto/credentials.json` and `config.json` are outside the workspace and
  therefore unreachable
- No process isolation — these are async `tokio::fs` calls in the quecto process itself

### web_search

**Isolation: none beyond the tool's own construction-time URL binding.**

- Outbound hosts (`api.search.brave.com`, `api.duckduckgo.com`) are fixed at
  construction time — not agent-supplied
- No per-query rate limit or response-size cap
- Prompt injection via search results is the primary risk (malicious page content
  returned to the LLM context)

### message

**Isolation: none beyond channel target validation.**

- Writes to an `mpsc` channel consumed by `TelegramChannel`
- `target` field is agent-supplied; no per-target rate limit
- A looping agent could spam a Telegram chat until Telegram's own 429 rate limit
  kicks in

### cron

**Isolation: none beyond name uniqueness and zero-interval rejection.**

- No cap on total number of jobs
- No minimum interval floor enforced at the tool level (only zero is rejected)
- Each job runs through the full agent loop; resource consumption is bounded only
  by the cron executor's `exec_timeout_minutes`

### recall

**Isolation: none beyond the 256-entry recall-count tracking cap.**

- Returns spilled tool output verbatim — no size cap on content returned to the LLM
- Repeated recall of the same ID emits a `tracing::warn!` at count ≥ 3 but does
  not error; a stuck model can loop indefinitely

### spawn

**Isolation: agent-ID allowlist + `restrict_to_workspace` inheritance.**

- Child is a full `quecto agent` process at the same OS trust level as the parent
- Reads its own credentials from `~/.quecto/credentials.json` at startup (by design —
  it needs an LLM provider)
- `QUECTO_*` env vars are **not** explicitly stripped from the child's environment,
  but this is low risk: the child already reads credentials from disk via
  `CredentialStore`, and env-var keys are redundant
- No spawn-depth limit; a child can spawn grandchildren

---

## Covered threats

| Threat | Mitigation |
|---|---|
| bash reads `credentials.json` | Not mounted in nsjail; workspace boundary excludes `~/.quecto/` |
| bash sees `QUECTO_*` API keys via env | Stripped at two independent points (`SECRET_ENV_PREFIX`) |
| bash makes outbound network calls | Network namespace disabled by default |
| Filesystem tool escapes workspace via symlink | `canonicalize()` + prefix check in `Sandbox::validate_path()` |
| Filesystem tool reads `credentials.json` | File is above workspace root — blocked by sandbox |
| bash runs destructive commands | Denylist (`rm -rf /`, `mkfs`, `dd if=/dev/zero`, etc.) always checked first |
| bash runs arbitrary commands when allowlist set | All tokens across shell metacharacters validated |

## Known gaps (low severity)

| Gap | Notes |
|---|---|
| `web_search` response size unbounded | Large responses inflate LLM context; prompt injection surface |
| `message` has no rate limit | Looping agent can spam until Telegram 429s |
| `cron` has no job count cap or minimum interval | Resource exhaustion via many high-frequency jobs |
| `recall` has no content size cap | Large spill entries re-injected verbatim into context |
| `spawn` has no depth limit | Unbounded recursion possible |

None of these gaps allow credential exfiltration or host compromise. They are
resource-consumption and LLM-context quality concerns.

---

## What was removed in #194 (WASM) and why it doesn't matter

The WASM infrastructure included capability declarations, HTTP allowlisting,
fuel metering, and memory limits. However, none of these were ever wired into
the composition root — they were never active at runtime. Removing WASM did not
change the actual security posture. The isolation that matters (nsjail for bash,
`Sandbox` for filesystem tools) was always separate from WASM and remains in place.
