# Container runtime (runbook): init → build → doctor → spawn → get_containers → kill

Subagents run in containers through the `container_configs` entries of the
effective configuration (global file plus the working directory's trusted
`.quecto/config.json` overlay). Each entry names the script argv that
creates, joins, inspects, kills and cleans up an environment; the official
Docker/Podman adapter is `scripts/container-runtime/docker/*.sh`, and
`quecto container init` materialises it for a repository. This page is the
operating runbook; `docs {"name": "subagents"}` covers how to spawn.

## Preconditions

- You are in the repository **root** (`git rev-parse --show-toplevel` = `pwd`); init refuses a subdirectory, naming the root (`--project <root>` from elsewhere).
- Rootless `podman` (preferred) or `docker` ≥ 20.10 on PATH, plus `jq` and `git`; `gh` logged in (`gh auth status`) if children must push or use the GitHub API (without it: a warning, no token inside).
- The repository's `origin` remote is a URL the host can clone with its own credentials (`git ls-remote --exit-code origin` exits 0), with **no credential embedded** (init refuses `https://user:secret@…` and `https://ghp_…@…`). No remote = a sandbox entry (empty workspace), which init says.
- `quecto status` shows `Overlay: none` or `(trusted)`. An `(untrusted)` overlay is refused by init whatever it declares: `quecto config trust` first (after review). `init` and `status` refuse an explicit `--config` (they work on this checkout's overlay); only `doctor` accepts one.
- Disk and network for one image build (~300 MB: Debian trixie-slim + git, gh, jq, curl, ripgrep, fd, python3; no toolchain).

## Do

1. **Initialise** — writes the bundle and binds the repository (idempotent):
   ```
   quecto container init                      # --repo from origin; --image quecto-box:local
   quecto container init --repo <url>         # explicit repository
   quecto container init --image <tag>        # another image tag
   quecto container init --dry-run            # print what would be written
   ```
   Expected (`wrote` per file on a first run, `kept` on a re-run):
   ```
   standard container bundle (version 2) at /repo/.quecto/containers/standard
     wrote  /repo/.quecto/containers/standard/Containerfile
     wrote  /repo/.quecto/containers/standard/scripts/create.sh
     wrote  … exec.sh, inspect.sh, kill.sh
   container config "standard" written as container_configs.standard in /repo/.quecto/config.json (trusted for exactly these bytes)
     default: true — `spawn container: true` selects it
     --repo https://github.com/org/app.git (the checkout's origin remote): a new container is a fresh clone of it
     --state-dir under the quecto base directory; --image quecto-box:local
   next:
     1. build the image … podman build -t quecto-box:local -f /repo/.quecto/containers/standard/Containerfile /repo/.quecto/containers/standard
     2. quecto container doctor   — every check ✓
     3. spawn {"agent_id":"probe","task":"run pwd","container":true} …
   ```
   If another entry is already the default, init adds `standard` without the label and says so (select it with `container: {"mode":"new","container_config":"standard"}`, diagnose with `doctor --name standard`). A re-init keeps the entry's `--repo`/`--image` unless the flag is given (`kept:`/`rewrote:` lines).
2. **Build the image** — exactly the command init printed (a create never builds or pulls). Skip it when `quecto container status` already reports `image: image quecto-box:local is present`:
   ```
   podman build -t quecto-box:local -f /repo/.quecto/containers/standard/Containerfile /repo/.quecto/containers/standard
   ```
   Expected: the last line is `Successfully tagged localhost/quecto-box:local` (docker: `naming to docker.io/library/quecto-box:local`). A docker-only host runs the same command with `docker`: the scripts drive whichever runtime the doctor's `runtime-cli` line names. A project that needs a toolchain derives `FROM quecto-box:local` and passes `--image <tag>` to init.
3. **Use** — from an agent started in this repository:
   `spawn {"agent_id":"probe","task":"run pwd and git log -1 --oneline, then exit","container":true}` → the result names `environment_ref=C1 container_config=standard`; `agent_cmd {"agent_id":"*","command":"get_containers"}` lists it `running` with the repository; `agent_cmd {"agent_id":"*","command":"kill_container","ref":"C1"}` ends it (its members are terminated, the config's kill runs once; from the shell, `quecto container kill C1`). When the last member of an ordinary environment exits it tears itself down; a swarm container is `retained` until `kill_container` / `quecto container kill`.

## Verify

```
quecto container status
```

Expected (exit 0; exit 1 — last line `not ready: …` — while any line is not in place, *including* a deliberately edited Containerfile, see Trust boundary):

```
standard container at /repo/.quecto/containers/standard
  assets:  present (5 of 5, version 2)
  config:  standard (default, overlay) in the effective configuration; --repo https://github.com/org/app.git
  trust:   trusted (the repo-local overlay is applied)
  image:   image quecto-box:local is present
ready: spawn {"container":true} from an agent in this project
```

```
quecto container doctor [--name <config>] [--config <file>]
```

It resolves the effective config exactly as `spawn container: true` does, runs the create script's own preflight (`create.sh --preflight-only`) **without creating anything**, one line per check, exit 0 when none failed (warnings allowed):

```
container config "standard" (create: /repo/.quecto/containers/standard/scripts/create.sh --state-dir /home/me/.quecto/container-environments --repo https://github.com/org/app.git --image quecto-box:local)
  ✓ runtime-cli  podman at /usr/bin/podman
  ✓ jq           jq at /usr/bin/jq
  ✓ git          git at /usr/bin/git
  ✓ gh           gh at /usr/bin/gh
  ✓ image        image quecto-box:local is present
  ✓ repo         --repo https://github.com/org/app.git is reachable
  ✓ state-dir    state dir /home/me/.quecto/container-environments is writable and owned by the current user   (or: will be created under writable /home/me/.quecto)
0 checks failed, 0 warnings
```

Then the spawn in Do step 3: `environment_ref=C1 container_config=standard`, and `get_containers` shows `status: running`.

## Rollback

```
quecto config unset --local container_configs.standard   # → unset container_configs.standard in /repo/.quecto/config.json (trusted)
rm -r .quecto/containers/standard
podman rmi quecto-box:local                              # optional: the image
```

Running environments are unaffected until killed (`kill_container`, or `quecto container kill <ref|name>` from the shell; `quecto container gc` removes exited leftovers). Other repositories are never affected: the overlay is read from the working directory only.

## If it fails

| Symptom | Cause | Fix |
|---|---|---|
| init: `… is not the repository root (<root>): … run init from the root, or pass --project <root>` | run from a subdirectory | `cd` to the root, or pass `--project` |
| init: `overlay … is not trusted …` | hand-written or edited overlay | review `quecto config get --local`, `quecto config trust`, retry |
| init: `… carries a credential in its userinfo …` | userinfo in the URL | `git remote set-url origin <credential-free url>` or `--repo <url>` |
| doctor `✗ runtime-cli` | no podman/docker on PATH, or the daemon not answering | install/start it; `QUECTO_CONTAINER_CLI=/abs/podman` overrides the choice |
| doctor `✗ image  image quecto-box:local is not present …` | step 2 skipped (create never pulls) | run the printed `podman build …` |
| doctor `✗ repo  --repo <url> is unreachable: …` | wrong URL, no network, or missing credentials (`repository … not found` for a private repo) | `gh auth login` / ssh key, or `quecto container init --repo <url>` with the right URL |
| doctor `! gh  gh is not on PATH …` (warning) | children get no GitHub token | install `gh`, `gh auth login`; harmless if children never push |
| doctor `✗ state-dir` | `<base_dir>/container-environments` not writable | fix ownership/permissions of the base dir |
| spawn error `script-managed create failed with status exit status: N: …` | the create's own failure, stderr tail quoted | read the tail; shipped codes 2–8 are the preflight checks (2 usage, 3 runtime, 4 jq, 5 git, 6 image, 7 repo, 8 state dir — the doctor shows the same); 1 = a later step (clone, `podman run`), environment already rolled back |
| spawn error `container config 'standard' refused: …/scripts/create.sh differs from the standard bundle this quecto embeds …` | a script under `.quecto/containers/standard/` was edited, pulled, or written by an older quecto | `git diff .quecto/containers/standard`, then `quecto container init --refresh` |
| `container: true refused` with `overlay_withheld: true` in `get_container_configs` | untrusted/refused overlay | `diagnostics` names it; `quecto config trust`, retry |
| `unknown container config` | name not in the effective set | `agent_cmd get_container_configs` lists the live names |
| `does not support --preflight-only` | a custom create script predating the preflight contract | add the mode to the script (both shipped sets implement it) |
| the child sees an empty workspace or an old default branch | a new container is a fresh clone of `--repo` at its default branch | push your branch and tell the child to `git fetch && git checkout <branch>` |

## Trust boundary

The overlay's trust covers `.quecto/config.json`, not the scripts it names, and those scripts run **on the host** before any container exists. So the program (first argv element) of every argv the host is about to run is compared with the bytes this binary embeds whenever it lies under `.quecto/containers/standard/`: every script the entry names at create (`spawn container: true`, `container doctor`), the retained exec argv at a join, and the retained inspect/kill/cleanup argv before each runs. One that differs, is missing or is a symbolic link is refused before it runs; a refused kill leaves `cleanup-failed` with the same reason (retry after the refresh); `status` lists the file as `differs`. Review a pulled change to `.quecto/containers/standard/` as you would one to `.quecto/config.json` (`git diff`), then `quecto container init --refresh`. The Containerfile is not run on the host and not checked at launch; its base is tag-pinned (`debian:trixie-slim`) — pin a digest in the materialised file if the build must be reproducible — `status` then lists it as `differs` and exits 1 (`not ready`), which is expected: rely on `doctor` exit 0 and the `image:` line instead, and re-apply the pin after any `init --refresh` (which replaces every differing file, the Containerfile included).

## Upgrades

The comparison is against *this binary's* bundle, so after upgrading quecto every changed script reads as `differs` and launches are refused with the message above. Run `quecto container init --refresh` in each checkout (each differing file → `refreshed`; a digest-pinned Containerfile is replaced too — re-pin it), rebuild the image if the Containerfile changed, `quecto container doctor`, then spawn. Environments created before the refresh are torn down by the refreshed scripts.

## How to find configs and refs

- **Which configs exist for this checkout** (global file plus the trusted overlay): the `spawn` tool description carries one line — `Available container configs: standard (default, repo-bound), quecto (global), …` (`repo-bound` = declared by this repository's overlay; `+N more` folds a long list; `(repo overlay untrusted — run quecto config trust)` means `container: true` is refused until then). Live detail: `agent_cmd {"agent_id":"*","command":"get_container_configs"}` → `{"container_configs":[{"name","default","source":"overlay"|"global","repository","problem","joinable"}],"overlay_withheld":bool,"diagnostics":[…]}` — the `container: true` default first; `repository` is what a new container clones (`null` = sandbox). Operators: `quecto config get --effective container_configs`.
- **Which environments are running** (for `{"mode":"existing"}` joins and `kill_container`): `agent_cmd {"agent_id":"*","command":"get_containers"}` → `containers[]` with `ref` (`C1`, durable per base dir, never reused), `name`, `status`, `workspace`, `repository`, `members`. A spawn result names its ref and config: `environment_ref=C1 container_config=<name>`.
- **What a new container is**: a fresh clone of the config's `--repo` at its default branch. Your working tree, branch and uncommitted changes are not inside; push a branch and tell the child to fetch/checkout it.

## Environments from earlier sessions

Refs and names are durable per base dir (`<base_dir>/environments.json`):
a container created by an earlier or concurrent session survives a
restart and appears in `get_containers` with `restored: true` and
`session` (its creator), status `empty` (its members are not reachable
here), `retained`, or `stopped` (its container was gone when this session
started; other sessions' later changes show after a restart). Join one
that is `empty`/`retained` with `container: {"mode":"existing","ref":"C1"}`
(your joiner leaving never tears it down); stop it with `kill_container`
(`ref` or `name`) — that cuts off any live members of its creating
session. A `name` still naming a live environment is refused at create.
From the shell:

```
quecto container ls [--all]        # live environments (--all: stopped too)
quecto container kill <ref|name>   # retained kill, record → stopped
quecto container gc --dry-run      # orphans of this config: container gone/exited AND no record or a stopped one
quecto container gc [--name <config>]   # remove them (the config's inspect / inspect --list / cleanup)
```

`gc` keeps a `running`/`retained`/`cleanup-failed` record, a state dir
whose container runs or cannot be checked, and a directory younger than
15 minutes without a container (a create in flight); it reports what it
removed, kept and why. Clean up after yourself: kill what you created
when done, and run `quecto container gc --dry-run` when `ls` shows
stopped leftovers.

## See also

- `docs {"name": "subagents"}` — spawning, joining, killing environments
- `docs {"name": "config"}` — overlay, trust, hand-rolled entries
- `docs {"name": "swarm"}` — a swarm runs in the container its coordinator was spawned into
- Full human reference (not embedded): `docs/container-runtimes.md` in the repo
