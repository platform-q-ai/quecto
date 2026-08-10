# ADR-0022: Self-Contained Named Container Configs

**Status:** Accepted

## Context

The #1369 epic delivered script-managed container spawning; ADR-0021 fixed
its architecture (ports, layers, registry). Using it exposed an interaction
problem (#1410): Quecto resolved a repository *before* the configured
script ran (spawn `repo` argument, else the parent checkout's
`remote.origin.url`, else refusal), so the same request meant different
things in different directories, a parent outside a checkout could not
spawn a container at all, and agents could not discover which script sets
existed. This ADR records the contract-level decisions that replaced that
model (landed via PR #1412); ADR-0021's revision note records the same
change from the architecture side.

## Decision

- **A container config is a complete, self-contained definition of a
  working context.** Each named `container_configs` entry owns its source
  and credentials: the repository URL is baked into the config's own argv
  (e.g. `create.sh --repo <url>`), and any auth the fetch needs is
  script-owned (e.g. the Docker set's `gh auth token` bootstrap). Quecto
  neither discovers nor passes source information; the spawn `repo` field
  and `QUECTO_CONTAINER_REPO` do not exist.
- **The parent's location is irrelevant to container semantics.** "Spawn
  the quecto container" means the same thing from any directory. The
  parent's disk state (branch, uncommitted work) never leaks into an
  environment; fresh-from-remote is the semantics, and needing a specific
  branch is an instruction to the agent inside, not spawn plumbing.
- **Sandbox configs are first-class.** A config with no baked repository
  creates an empty workspace; the spawn result says so.
- **Default is an entry label.** Exactly one config carries
  `"default": true`, validated at config load (zero or multiple label
  errors fail there, enumerating names — not at spawn time). Explicit
  selection (`container_config: "<name>"`) never consults labels, but is
  reachable only through a config that passed load validation.
- **Agents can see the menu.** The spawn tool description carries a
  session-start roster of the available container configs with the default
  marked. The roster is a deliberate snapshot: the config file is re-read
  at spawn time and selection errors enumerate the live names, so
  enumerate-on-error is the runtime source of truth when the file has
  changed since session start.
- **Selection errors teach.** Unknown-name and label-validation errors
  list the configured names so an agent can offer the menu and confirm
  instead of dead-ending.
- **Vocabulary follows the contract, not the mechanism.** The key is
  `container_configs` (not "scripts") because resolving to a script set is
  today's implementation, not the concept; the legacy `container_scripts`
  key fails config load with rename guidance rather than degrading
  silently. User-facing language standardizes on **container**; the
  "environment" vocabulary remains internal (EnvironmentRegistry et al.)
  to avoid collision with dev/prod/test environment language.
- **Local spawning is untouched.** `container` omitted or `false` remains
  the default local child process — a first-class use case (edge
  deployments where containers are too bulky), not a fallback.

## Alternatives considered

- **Per-spawn `repo` with parent-checkout discovery** (the slice-1 model):
  rejected — puts source resolution in the wrong place and couples request
  meaning to the parent's location.
- **Cloning the parent's local checkout as the default source**:
  considered and rejected in the #1410 discussion — the parent may sit
  anywhere, entirely unrelated to the task, and layering
  committed-vs-uncommitted copy semantics added complexity for a source
  policy the configs express more honestly.
- **First-entry-in-file as the default**: rejected — JSON object order is
  not a reliable ordering; the explicit label is deterministic and travels
  with the entry between config files.

## Consequences

Working on an arbitrary new repository means adding a container config
first (or instructing the agent to clone inside its container). Repo-local
config overrides reintroduce location sensitivity for config *discovery*
only and require a trust gate before repo-supplied argv may execute
(#1409). The roster snapshot's staleness contract matches every other
startup-loaded setting.
