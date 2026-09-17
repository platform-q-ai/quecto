# Standard container runtime

The standard runtime is a **rootless Podman-only** environment for local Linux.
It is not Docker-compatible and never falls back to a host-local process. macOS,
Windows, remote Podman connections, Podman machines, and rootful Podman are
rejected for standard launch; Podman machine setup is operator information, not
support for this runtime.

## Explicit approval and binary-only use

The released Quecto binary embeds the versioned standard configuration,
Containerfile, and lifecycle scripts. You may initialize an explicitly selected
project without a checkout:

```text
quecto container init --project /absolute/project/root
```

Initialization is intentional and safe: it creates missing project-local files,
never overwrites edited files, rejects symlink/traversal targets, and is
idempotent. Review generated files before approving them. A binary copied out
of the source repository still has all standard assets. Initialization does not
edit global configuration, trust unrelated repository files, install Podman,
or build an image.

Building an image is a separate explicit approval. Verify the pinned Debian
slim base and recipe/input digest, then build in the isolated approved context.
A normal launch uses an already-approved image identity; missing images and
changed inputs fail closed rather than pulling, rebuilding, or retagging.

## Launch and isolation

Select the generated standard config explicitly (for example with the normal
container configuration selection mechanism), then use:

```text
container: {"mode":"new","container_config":"standard"}
container: {"mode":"existing","ref":"C1"}
```

The adapter mounts only the selected project workspace, private per-environment
home/state, private socket sidecars, exact Quecto binary (read-only), sanitized
selected config (read-only), explicitly selected API-key material, and the
admission client directory when authorized. It never mounts the host
`~/.quecto` tree, authority sockets/journal/admin/token, host PID/network,
Docker sockets, SSH agent, or unrelated credentials. OAuth refresh state uses a
narrow dedicated mediation/credential-store path with atomic replacement and
cross-process locking; it is not copied as rotating token snapshots and does
not downgrade OAuth to API-key-only authentication. Provider bindings are
preserved without exposing secret values in argv, inspect output, metadata, or
logs. API-key grants are opt-in and selected only by the provider allowlist.

The create operation mints an environment ID distinct from the session ref.
Join, inspect, kill, and cleanup use the recorded environment ID and retained
configuration. `kill` and `cleanup` are the only accepted destructive
operations; malformed operation and environment identifiers fail before any
removal. Cleanup is idempotent and removes only state owned by that environment.

## Tools and customization

The image is intentionally small: bash, certificates, core utilities, findutils,
grep/sed, curl, jq, less, git, OpenSSH client, `gh`, `rg`, `fd`, Python 3,
`venv`, and pip in the dedicated virtual environment. It contains no Rust,
Cargo, compiler, development headers, Node/npm, or Claude tooling. Customize a
project only by editing the generated local assets and then re-running the
explicit approval/build flow; do not execute arbitrary ambient project scripts.
DocsTool pages are embedded and can be read from any working directory.

## Troubleshooting

* **unsupported platform** — use an approved local Linux rootless Podman host;
  standard does not silently switch runtimes.
* **Podman unavailable** — install/configure Podman as an operator, then rerun;
  Quecto does not install it. Check `podman info` and rootless user namespaces.
* **image unavailable or stale** — perform the deliberate approved build and
  verify its identity; launch will not build automatically.
* **permission or socket failure** — check ownership/mode of private state and
  the exact authorized admission client directory. Authority state must remain
  inaccessible.
* **OAuth refresh failure** — preserve the host credential store and lock
  mediation; do not copy access/refresh tokens into project files or replace
  OAuth with an API key. A failed refresh leaves the last valid atomic store
  intact and is surfaced to the caller.

A reproducible prerequisite check is available on an operator Linux host:

```bash
CONTAINER_RUNTIME_CONFIG=/absolute/path/to/config.json \
  scripts/container-runtime/smoke.sh
```

It is non-destructive and reports only Podman usability. Follow it with the
explicit image build and lifecycle smoke matrix (create, join, inspect, kill,
cleanup, repeated cleanup, admission, selected auth, and forbidden-tool checks).
Do not claim a mocked check is an end-to-end Podman result.
