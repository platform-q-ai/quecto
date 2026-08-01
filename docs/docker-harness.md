# Docker harness for local TUI development

Run the Quecto harness inside a container while driving it from the
`quecto-tui` installed on your host. The container clones Quecto into a Docker
named volume, `cargo install`s the harness from that checkout, and starts
`quecto agent --mode uds`. The TUI attaches over a bind-mounted Unix socket.

The only host write is a temporary socket directory, removed on exit. The
development checkout and Quecto state live in Docker named volumes.

## Prerequisites

- Docker with Compose v2 (`docker compose version`)
- `quecto-tui` on your `PATH` (or set `QUECTO_TUI_BIN=/path/to/quecto-tui`)
- SSH access to the Quecto repo, since the container clones it. `~/.ssh` is
  mounted read-only and `SSH_AUTH_SOCK` is forwarded when available.
- `socat` on the host, only for `tcp-proxy` transport (Docker Desktop)

Provider credentials and config are seeded from `~/.quecto/config.json` and
`~/.quecto/credentials.json`. Without at least one provider configured the
agent exits with `no LLM providers configured` before the socket appears.

## Usage

```bash
scripts/docker-harness-local-tui.sh              # build image, then run
scripts/docker-harness-local-tui.sh --no-build   # skip the image build
scripts/docker-harness-local-tui.sh --api        # also start quecto-api
```

The first run for a given instance clones the repo and builds the harness in
release mode, which takes a few minutes. Later runs reuse the warm volume and
start in seconds. See `--help` for the full flag and environment reference.

## Running several harnesses at once

Each harness is an *instance*, and an instance owns its Docker volumes. The
instance name defaults to a slug of the checked-out ref, so working on
different branches isolates automatically:

```bash
QUECTO_REPO_REF=feature/a scripts/docker-harness-local-tui.sh
QUECTO_REPO_REF=feature/b scripts/docker-harness-local-tui.sh
```

To run several harnesses off the same ref, name them explicitly:

```bash
scripts/docker-harness-local-tui.sh --instance a
scripts/docker-harness-local-tui.sh --instance b
```

Instance `NAME` gets `quecto-workspace-NAME` (the `/workspace` checkout and
cargo install root) and `quecto-home-NAME` (Quecto state, including saved
sessions). Sockets are already unique per run, and each run gets its own
Compose project, so nothing else needs configuring.

Two live harnesses must not share a workspace volume — both entrypoints would
`checkout` and `pull` the same git tree out from under each other. Startup
refuses when another running container already holds the volume. Override with
`QUECTO_ALLOW_SHARED_WORKSPACE=1` if you are certain.

Reuse a small, stable set of instance names. Every new name means a cold
`cargo install` and its own `target/` directory, which costs minutes and
gigabytes; reused names start in seconds.

### Ports

Plain UDS publishes no host ports, so instances never collide. Ports only
matter with `--api` or `tcp-proxy`. Unpinned ports advance past anything
already bound, so parallel instances land on 8080, 8081, 8082 and so on. Ports
pinned with `--api-port` / `--proxy-port` (or the matching environment
variables) are honoured exactly and fail loudly when busy.

## Sessions

Conversations are saved under `/home/appuser/.quecto/sessions` in the
instance's home volume and survive restarts. Inside the TUI, `/resume` opens a
picker of saved chats, `/resume <name>` reopens a named `cli:<name>` session,
and `/new` starts a fresh one. There is no command-line flag to launch
directly into a previous conversation.

Sessions are per-instance: instance `b` cannot resume a conversation started
in instance `a`. Container sessions are also separate from any sessions in
your host's `~/.quecto`, since only config and credentials are seeded in.

## Which code actually runs

The container clones `QUECTO_REPO_URL` and checks out `QUECTO_REPO_REF`
(default `master`), then builds the harness from that checkout. Your local
working tree is used only for the Compose and Docker files, so local edits are
not visible to the agent. Set `QUECTO_REPO_REF` to run a different branch, and
note that `--instance` only names the volumes; it does not select a ref.

## Troubleshooting

**`Workspace volume ... is already in use`** — another harness is live on that
instance, or a container was orphaned when a run was killed before its cleanup
trap could run. Check with `docker ps`, then `docker rm -f <container>`.

**Stale checkout** — the entrypoint's `git fetch` is tolerant of failure, so if
SSH credentials are unavailable the container silently builds whatever is
already in the volume. To confirm what an instance will run:

```bash
docker run --rm -u "$(id -u):$(id -g)" --entrypoint bash \
  -v quecto-workspace-NAME:/workspace quecto-harness:local \
  -c 'git -C /workspace/quecto log --oneline -1'
```

**Entrypoint changes have no effect** — `scripts/docker-harness-entrypoint.sh`
is copied into the image, so rebuild without `--no-build` after editing it.
