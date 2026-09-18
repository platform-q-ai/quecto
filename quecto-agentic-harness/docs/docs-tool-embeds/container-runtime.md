# Container runtime (runbook)

Subagents run in containers through the `container_configs` entries of the
effective configuration (global file plus the working directory's trusted
`.quecto/config.json` overlay). Each entry names the script argv that
creates, joins, inspects, kills and cleans up an environment; the official
Docker/Podman adapter is `scripts/container-runtime/docker/*.sh`. This page
is the operating runbook; `docs {"name": "subagents"}` covers how to spawn.

## Preconditions (what a create needs)

- A container runtime on PATH: rootless `podman` (preferred) or `docker`
  (`QUECTO_CONTAINER_CLI` overrides the choice).
- `jq` and, for a config with `--repo`, `git`; `gh` is optional (without it
  members get no GitHub token, so pushes and the GitHub API fail inside).
- The image (`--image <img>` in the create argv, or `QUECTO_DOCKER_IMAGE`,
  default `quecto-box:local`) **present in the local store**: a create never
  pulls implicitly.
- The `--repo` URL reachable with your credentials (`git ls-remote`).
- A `--state-dir` you own and can write.

## How to find configs and refs

- **Which configs exist for this checkout** (global file plus the trusted
  `.quecto/config.json` overlay): the `spawn` tool description carries one
  line — `Available container configs: r (default, repo-bound), quecto (global), …`
  (`repo-bound` = declared by this repository's overlay; `+N more` folds a
  long list; `(repo overlay untrusted — run quecto config trust)` means
  `container: true` is refused until then). For detail, live:
  `agent_cmd {"agent_id":"*","command":"get_container_configs"}` →
  `{"container_configs":[{"name","default","source":"overlay"|"global","repository"}],"overlay_withheld":bool,"diagnostics":[…]}`
  — the `container: true` default first; `repository` is what a new
  container clones (`null` = sandbox). Operators: `quecto config get --effective container_configs`.
- **Which environments are running** (for `{"mode":"existing"}` joins and
  `kill_container`): `agent_cmd {"agent_id":"*","command":"get_containers"}`
  → `containers[]` with `ref` (`C1`, session-scoped), `name`, `status`,
  `workspace`, `repository`, `members`. A spawn result names its ref and
  config: `environment_ref=C1 container_config=<name>`.
- **What a new container is**: a fresh clone of the config's `--repo` at its
  default branch. Your working tree, branch and uncommitted changes are not
  inside; push a branch and tell the child to fetch/checkout it.

## Diagnose: `quecto container doctor`

```
quecto container doctor [--name <config>]        # from the repository's directory
quecto container doctor --config <file>           # that file's entries instead
```

It resolves the effective config exactly as `spawn container: true` does
(overlay-aware; `--name` picks an entry), runs the create script's own
preflight (`create.sh --preflight-only`) **without creating anything** (not
even the state dir), and prints one line per check with a remedy for every
`✗`/`!`:

```
container config "quecto" (create: /…/create.sh --state-dir /… --repo https://…)
  ✓ runtime-cli  podman at /usr/bin/podman
  ✓ jq           jq at /usr/bin/jq
  ✓ git          git at /usr/bin/git
  ! gh           gh is not on PATH: members will have no GitHub token (GH_TOKEN)
    remedy: install gh and run 'gh auth login' so members can push and use the GitHub API
  ✗ image        image quecto-box:local is not present in the local podman store
    remedy: build it (podman build -t quecto-box:local <dir with its Containerfile>) or pull it (podman pull quecto-box:local); create never pulls implicitly
  ✓ repo         --repo https://… is reachable
  ✓ state-dir    state dir /… is writable and owned by the current user
1 check failed, 1 warning
```

Exit code 0 when no check failed (warnings allowed), 1 otherwise. Apply the
remedy, run it again, then retry the spawn. `unknown container config`
lists the live names; a `does not support --preflight-only` error means the
config's create script predates the preflight contract (both shipped script
sets implement it).

## When a spawn fails

The tool error carries the script's last stderr lines after its status:
`script-managed create failed with status exit status: 6: … image
quecto-box:local is not present …`. The same text reaches the harness's
stderr. Read the message, run `quecto container doctor` in the same
directory, fix what it names. The shipped create scripts' exit codes:
2 usage, 3 runtime missing or not answering, 4 no jq, 5 no git, 6 image
missing, 7 `--repo` unreachable, 8 state dir. A create that failed **after** its preflight
(clone, `podman run`) rolled its environment back; `kill.log` in the state
dir records every kill/cleanup.

## Verify

`spawn {"agent_id":"probe","task":"run pwd and exit","container":true}`
returns `environment_ref=C1 container_config=<name>`; `agent_cmd
get_containers` (`agent_id: "*"`) lists it as `running`.

## See also

- `docs {"name": "subagents"}` — spawning, joining, killing environments
- `docs {"name": "config"}` — binding a repository to a config (overlay, trust)
- Full human reference (not embedded): `docs/container-runtimes.md` in the repo
