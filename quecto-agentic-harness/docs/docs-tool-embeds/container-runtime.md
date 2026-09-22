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
- Disk and network for one image build (the starter is under 1 GB: Debian trixie + git, gh, jq, curl, ripgrep, fd, python3, build-essential; no language toolchain — the project's own Containerfile adds its own).

## Do

1. **Initialise** — writes the bundle and binds the repository (idempotent):
   ```
   quecto container init                      # --repo from origin; --image quecto-<folder>:local (here /repo → quecto-repo:local)
   quecto container init --repo <url>         # explicit repository
   quecto container init --image <tag>        # another image tag
   quecto container init --dry-run            # print what would be written
   ```
   Expected (`wrote` per file on a first run, `kept` on a re-run):
   ```
   standard container bundle (version 5) at /repo/.quecto/containers/standard
     wrote  /repo/.quecto/containers/standard/Containerfile
     wrote  /repo/.quecto/containers/standard/scripts/create.sh
     wrote  … exec.sh, inspect.sh, kill.sh
   container config "standard" written as container_configs.standard in /repo/.quecto/config.json (trusted for exactly these bytes)
     default: true — this repo's default: `spawn container: true` selects it in this repo (no default elsewhere overrides a repo's standard container)
     --repo https://github.com/org/app.git (the checkout's origin remote): a new container is a fresh clone of it
     --state-dir under the quecto base directory; --image quecto-repo:local
   next:
     first: /repo/.quecto/containers/standard/Containerfile is still the neutral starter — add this project's toolchain …
     1. build the image … podman build -t quecto-repo:local -f /repo/.quecto/containers/standard/Containerfile /repo/.quecto/containers/standard
     2. quecto container doctor   — every check ✓
     3. spawn {"agent_id":"probe","task":"run pwd","container":true} …
   ```
   **`standard` is always this repo's default.** A global default (`the global default <name> does not apply in this repo`) is overridden here only — the global file is untouched, other repos keep it. Another overlay entry carrying `"default": true` loses the label in the same write (`displaced default: <name>`; select it by name). Launch policy applies the rule, not just the label: a repo-bound `standard` is what `container: true` selects even after `quecto config unset --local container_configs.standard.default` removed its label (possible only while another entry is labelled; a raw edit un-trusts the overlay instead) or the global file was re-labelled — `status` then says `default by rule` with the remedy (`init --refresh` or `quecto config set --local container_configs.standard.default true`). Init names what it displaced from the *effective* set: an overlay entry's label (`displaced default:`), else the global label it overrides. A re-init keeps the entry's `--repo`/`--image` unless the flag is given (`kept:`/`rewrote:`/`added:` lines).
   The `first:` line is printed for as long as the Containerfile is byte-for-byte the starter, whichever run wrote it; it goes once the project has made the file its own.
2. **Make the Containerfile this repo's** — skip only when the Containerfile already carries this repo's toolchain (an uncommented toolchain section or an uncommented `LABEL ai.quecto.required-tools` line — the starter's example is a comment); then *read* it, since its `RUN` steps execute in your build. An untouched starter is not that, whoever wrote it and whenever: the container is only useful once it holds what this repo builds and tests with:
   - Read the repo's manifests (`Cargo.toml`, `pyproject.toml`, `package.json`, `go.mod`, …) and its CI configuration: languages, the tools CI runs, the versions the repo pins.
   - In the starter's marked section of `.quecto/containers/standard/Containerfile`, install that toolchain (pinned where the repo pins it) and declare what an agent must find on PATH: `LABEL ai.quecto.required-tools="python3 uv pytest ruff"`.
   - Install tools only: never `COPY` the repo's source and never put a token, key or private index URL in the file — the checkout is mounted at run time. With several toolchains, or none you can identify, ask which to include.
   - Show the file and ask before writing it. Commit it afterwards — the next agent in this folder then builds the same container. `quecto container init --refresh` never replaces it.
3. **Build the image** — exactly the command init printed (a create never builds or pulls). Skip it only when `quecto container status` reports `image: image quecto-repo:local is present` **and** the Containerfile has not changed since that build; after step 2, always rebuild — the doctor would otherwise judge the old image:
   ```
   podman build -t quecto-repo:local -f /repo/.quecto/containers/standard/Containerfile /repo/.quecto/containers/standard
   ```
   Expected: the last line is `Successfully tagged localhost/quecto-repo:local` (docker: `naming to docker.io/library/quecto-repo:local`). A docker-only host runs the same command with `docker`: the scripts drive whichever runtime the doctor's `runtime-cli` line names. **The image tag is the project's too**: without `--image`, init names it after the project folder — `quecto-<folder>:local` (lowercased; ASCII letters/digits kept, a single `.`/`_` kept, other runs `-`; ≤100 chars; nothing admissible → `quecto-dev:local`). Same-named folders share a tag and a git worktree is a different folder: pass `--image` to tell repos apart or to share the main checkout's image. A re-init keeps the tag the entry already has (one with no `--image` keeps `quecto-dev:local`). **The Containerfile is the project's**: init writes a neutral starter only where none exists; edit it (toolchain + label), commit it, rebuild. It is never drift and `--refresh` never replaces it — only the four scripts are quecto's. In a repo you cloned, read its Containerfile before building it. The doctor is tooling-neutral: it asks any image for a shell and `git` (`image-base`), plus exactly the tools the image itself declares in `LABEL ai.quecto.required-tools="python3 uv pytest"` (`required-tools`; bare ASCII names separated by whitespace; no label, no extra check; redeclare the label in a derived image). The image needs `ENTRYPOINT []` or an entrypoint that executes its arguments.
4. **Use** — from an agent started in this repository:
   `spawn {"agent_id":"probe","task":"run pwd and git log -1 --oneline, then exit","container":true}` → the result names `environment_ref=C1 container_config=standard`; `agent_cmd {"agent_id":"*","command":"get_containers"}` lists it `running` with the repository; `agent_cmd {"agent_id":"*","command":"kill_container","ref":"C1"}` ends it (its members are terminated, the config's kill runs once; from the shell, `quecto container kill C1`). When the last member of an ordinary environment exits it tears itself down; a swarm container whose run has not been closed is `retained` (resumable) until `kill_container` / `quecto container kill`.

## Verify

```
quecto container status
```

Expected (exit 0; exit 1 — last line `not ready: …` — while a script differs or anything is missing; the project's own Containerfile never counts, see Trust boundary):

```
standard container at /repo/.quecto/containers/standard
  assets:  present (5 of 5, version 5)
  config:  standard (default, overlay) in the effective configuration; --repo https://github.com/org/app.git
           this repo's default: `spawn container: true` selects it whatever the global file labels
  trust:   trusted (the repo-local overlay is applied)
  image:   image quecto-repo:local is present
ready: spawn {"container":true} from an agent in this project
```

```
quecto container doctor [--name <config>] [--config <file>]
```

It resolves the effective config exactly as `spawn container: true` does, runs the create script's own preflight (`create.sh --preflight-only`) **without creating anything**, one line per check, exit 0 when none failed (warnings allowed):

```
container config "standard" (create: /repo/.quecto/containers/standard/scripts/create.sh --state-dir /home/me/.quecto/container-environments --repo https://github.com/org/app.git --image quecto-repo:local)
  ✓ runtime-cli     podman at /usr/bin/podman
  ✓ jq              jq at /usr/bin/jq
  ✓ git             git at /usr/bin/git
  ✓ gh              gh at /usr/bin/gh
  ✓ image           image quecto-repo:local is present
  ✓ image-base      image quecto-repo:local provides a shell and git
  ✓ required-tools  image quecto-repo:local declares no required tools (ai.quecto.required-tools is not set)   (or: provides the tools it declares: …)
  ✓ repo            --repo https://github.com/org/app.git is reachable
  ✓ state-dir       state dir /home/me/.quecto/container-environments is writable and owned by the current user   (or: will be created under writable /home/me/.quecto)
0 checks failed, 0 warnings
```

Then the spawn in Do step 4: `environment_ref=C1 container_config=standard`, and `get_containers` shows `status: running`.

## Rollback

```
quecto config unset --local container_configs.standard   # → unset container_configs.standard in /repo/.quecto/config.json (trusted)
rm -r .quecto/containers/standard
podman rmi quecto-repo:local                              # optional: the image
```

Running environments are unaffected until killed (`kill_container`, or `quecto container kill <ref|name>` from the shell; `quecto container gc` removes exited leftovers). Other repositories are never affected: the overlay is read from the working directory only.

## If it fails

| Symptom | Cause | Fix |
|---|---|---|
| init: `… is not the repository root (<root>): … run init from the root, or pass --project <root>` | run from a subdirectory | `cd` to the root, or pass `--project` |
| init: `overlay … is not trusted …` | hand-written or edited overlay | review `quecto config get --local`, `quecto config trust`, retry |
| init: `… carries a credential in its userinfo …` | userinfo in the URL | `git remote set-url origin <credential-free url>` or `--repo <url>` |
| doctor `✗ runtime-cli` | no podman/docker on PATH, or the daemon not answering | install/start it; `QUECTO_CONTAINER_CLI=/abs/podman` overrides the choice |
| doctor `✗ image  image quecto-repo:local is not present …` | step 3 skipped (create never pulls) | run the printed `podman build …` |
| doctor `✗ image-base  … missing git` | the image lacks a shell or `git` | add them to the Containerfile, rebuild |
| doctor `✗ required-tools  … missing <tool>` / `… is not a tool name` | the image lacks a tool its `ai.quecto.required-tools` label declares, or the label is not a list of bare names | rebuild from the Containerfile, or correct the label |
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

The overlay's trust covers `.quecto/config.json`, not the scripts it names, and those scripts run **on the host** before any container exists. So the program (first argv element) of every argv the host is about to run is compared with the bytes this binary embeds whenever it lies under `.quecto/containers/standard/`: every script the entry names at create (`spawn container: true`, `container doctor`), the retained exec argv at a join, and the retained inspect/kill/cleanup argv before each runs. One that differs, is missing or is a symbolic link is refused before it runs; a refused kill leaves `cleanup-failed` with the same reason (retry after the refresh); `status` lists the file as `differs`. Review a pulled change to `.quecto/containers/standard/` as you would one to `.quecto/config.json` (`git diff`), then `quecto container init --refresh`. The Containerfile is not run on the host and not checked at launch; it is the project's own file — `status` lists it as `this project's own` and stays `ready`, and `init --refresh` never replaces it.

## Upgrades

The comparison is against *this binary's* bundle, so after upgrading quecto every changed script reads as `differs` and launches are refused with the message above. Run `quecto container init --refresh` in each checkout (each differing script → `refreshed`; the project's Containerfile is kept), `quecto container doctor`, then spawn. Environments created before the refresh are torn down by the refreshed scripts.

## How to find configs and refs

- **Which configs exist for this checkout** (global file plus the trusted overlay): the `spawn` tool description carries one line — `Available container configs: standard (default, repo-bound), quecto (global), …` (`repo-bound` = declared by this repository's overlay; `+N more` folds a long list; `(repo overlay untrusted — run quecto config trust)` means `container: true` is refused until then; a repo-bound `standard` is listed `default` whatever the labels say, because it is what `container: true` selects there). Live detail: `agent_cmd {"agent_id":"*","command":"get_container_configs"}` → `{"container_configs":[{"name","default","source":"overlay"|"global","repository","problem","joinable"}],"overlay_withheld":bool,"diagnostics":[…]}` — the `container: true` default first; `repository` is what a new container clones (`null` = sandbox). Operators: `quecto config get --effective container_configs`.
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
(your joiner leaving never tears it down) — `empty` means its creating
session is still alive (a member harness exits with its parent, and the
container with it), so this is the concurrent-session case; stop it with `kill_container`
(`ref` or `name`) — that cuts off any live members of its creating
session. A `name` still naming a live environment is refused at create.
From the shell:

```
quecto container ls [--all]        # live environments (--all: stopped too)
quecto container kill <ref|name>   # retained kill, record → stopped
quecto container gc --dry-run      # orphans of this config: container gone/exited AND no record or a stopped one
quecto container gc [--name <config>]   # remove them (the config's inspect / inspect --list / cleanup)
quecto container gc --abandoned-after 3d   # also collect unrecorded directories whose board still says a run is on, once 3 days old (--abandoned: regardless of age)
```

`gc` keeps a `running`/`cleanup-failed` record, a `retained` one
whatever its container's state (a retained swarm box has exited by
design; only `container kill` ends it), a state dir whose container runs
or cannot be checked, a directory younger than 15 minutes without a
container (a create in flight), and any stopped or unrecorded directory
whose checkout still hosts a swarm run its owner has not closed — running,
paused, paused holding an outcome, cancelled (`hosts swarm run <id>
(<status>)`; the board is the run's — kill explicitly, or end the run, to
collect). An unrecorded such directory is an *abandoned run*: `--abandoned`
collects them all, `--abandoned-after <Ns|Nm|Nh|Nd>` those at least that
old, each with the reason naming the run and the flag; it judges the config's `--state-dir` only (a
record's own retained cleanup may name one more root, for that record
alone), and `--dry-run` writes nothing — not even to `environments.json`.
It reports what it removed, kept and why. A `running` record whose box
exited with an unfinished swarm run inside (the master died before its
coordinator) is restored `retained`, never `stopped`. A container no
record names (a harness died between create and journal) exits with its
parent; while it runs, end it by hand: `podman rm -f
quecto-<environment-id>`. Clean up after yourself: kill what you created
when done, and run `quecto container gc --dry-run` when `ls` shows
stopped leftovers.

## See also

- `docs {"name": "subagents"}` — spawning, joining, killing environments
- `docs {"name": "config"}` — overlay, trust, hand-rolled entries
- `docs {"name": "swarm"}` — a swarm runs in the container its coordinator was spawned into
- Full human reference (not embedded): `docs/container-runtimes.md` in the repo
