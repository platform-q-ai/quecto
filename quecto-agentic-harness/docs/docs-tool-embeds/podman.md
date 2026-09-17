# Podman containers

Quecto's standard Podman adapter is the rootless, same-host container runtime.
It is selected with the `podman` container configuration; it does not require a
Podman daemon or a Docker socket. The canonical scripts are:

- `scripts/container-runtime/podman/create.sh`
- `scripts/container-runtime/podman/exec.sh`
- `scripts/container-runtime/podman/inspect.sh`
- `scripts/container-runtime/podman/kill.sh`

Use `container: {"mode":"new","container_config":"podman"}` to start one,
or join it with `{"mode":"existing","ref":"C1"}`. The configuration is
self-contained: repository and image arguments belong to its create command.
Do not add a nested container or bypass the configured runtime.

## Limits and lifecycle

The swarm pool is limited to **1–25 members total**, including its coordinator;
idle and reserved members count. This is an orchestration limit, not a Podman
limit. Podman environments are one container per environment, with the child
as the workload and a real init at PID 1. The standard create script applies a
pids limit of **16,384** by default; `QUECTO_CONTAINER_PIDS_LIMIT=-1` delegates
the limit to the user slice, while `0` is invalid. Threads consume this limit.

Rootless Podman uses a user namespace (`--userns=keep-id`) and identity mounts
for the workspace, socket directory, configuration, and `$HOME/.quecto`.
Existing project files are preserved. Provider and GitHub credentials are
passed through the adapter's protected, short-lived environment file; they
are not placed in inspectable container environment configuration.

For the complete wire contract, security rationale, cleanup behavior, and
manual verification procedure, read `docs/container-runtimes.md` with the
repository file tools only when working in the checkout. This page is the
agent-facing summary; the `docs` tool itself is embedded and CWD-independent.
