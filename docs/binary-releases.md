# Binary releases

Quecto publishes paired binary bundles for the `quecto` harness and the
`quecto-tui` terminal client through the **Binary Release** GitHub Actions
workflow (`.github/workflows/binary-release.yml`). This page is the maintainer
runbook and the user installation guide for those bundles.

## Release policy

Binary releases are intentionally explicit and narrow:

- A release is eligible only after a pull request targeting `master` is merged.
- The merged pull request must carry the `release-binaries` label at merge time.
- The automatic workflow uses `pull_request_target` only for closed, merged PRs
  so publishing runs from the trusted base context with write credentials while
  still building the exact PR merge commit recorded by GitHub, not unreviewed PR
  head code or whatever `master` points at later.
- Manual `workflow_dispatch` runs are allowed for retries and recovery, but they
  must be started from the `master` branch and must name an already-merged
  `master` PR that carried `release-binaries` at merge time; manual runs cannot
  publish an arbitrary commit or make a previously ineligible PR eligible by
  adding the label after merge.
- Releases are serialized with a single `binary-release` concurrency group.
  Another binary release will wait rather than race tag selection or asset
  publication.
- The build uses `cargo build --release --locked` so `Cargo.lock` is authoritative
  and the release cannot silently resolve newer dependency versions.
- The workflow grants repository contents write permission only to the final
  publish job. Eligibility and build jobs have read-only permissions.

## Tags

Release tags use the UTC calendar date:

```text
vYYYY.MM.DD
```

If a tag for the same UTC date already exists, the workflow appends a numeric
collision suffix:

```text
vYYYY.MM.DD.1
vYYYY.MM.DD.2
```

Retry behavior is idempotent for the same merge commit. If a prior attempt
already created a date-style tag that points at the intended commit, a manual
retry reuses that tag and uploads or replaces the release assets for it. If the
planned tag already exists but points at a different commit, the run fails rather
than moving the tag.

## Platforms and assets

Each release uploads four `.tar.gz` archives plus `SHA256SUMS.txt`:

| Platform | Asset name pattern | Rust target |
|---|---|---|
| Linux x86_64 | `quecto-vYYYY.MM.DD-linux-x86_64.tar.gz` | `x86_64-unknown-linux-gnu` |
| Linux ARM64 | `quecto-vYYYY.MM.DD-linux-arm64.tar.gz` | `aarch64-unknown-linux-gnu` |
| macOS Intel | `quecto-vYYYY.MM.DD-macos-intel.tar.gz` | `x86_64-apple-darwin` |
| macOS Apple Silicon | `quecto-vYYYY.MM.DD-macos-apple-silicon.tar.gz` | `aarch64-apple-darwin` |

For suffixed tags, the tag segment in the asset name includes the suffix, for
example `quecto-v2026.09.04.1-linux-x86_64.tar.gz`.

Every archive contains both executable files:

```text
quecto
quecto-tui
README.txt
```

The two binaries are built together from the same source commit and should be
installed together. `quecto-tui` starts the harness by executing `quecto` when no
`--socket` is supplied, so both binaries must be present on `PATH` for the
normal local launch flow.

Windows binaries are not produced. Windows users should run Quecto under WSL2
and install the Linux x86_64 bundle inside the WSL2 distribution.

## User installation

### 1. Choose the correct archive

Use `uname` to identify your platform:

```bash
uname -s
uname -m
```

Map the output to an asset:

| `uname -s` | `uname -m` | Download |
|---|---|---|
| `Linux` | `x86_64` | `linux-x86_64` |
| `Linux` | `aarch64` or `arm64` | `linux-arm64` |
| `Darwin` | `x86_64` | `macos-intel` |
| `Darwin` | `arm64` | `macos-apple-silicon` |

On Windows, first install and enter WSL2, then use the Linux row that matches
the WSL2 architecture. Most Windows-on-Intel/AMD installations use
`linux-x86_64`.

### 2. Download the archive and checksums

Replace `vYYYY.MM.DD` with the release tag you want:

```bash
TAG=vYYYY.MM.DD
ASSET=linux-x86_64
BASE="https://github.com/<owner>/<repo>/releases/download/${TAG}"

curl -LO "${BASE}/quecto-${TAG}-${ASSET}.tar.gz"
curl -LO "${BASE}/SHA256SUMS.txt"
```

When using this repository directly, replace `<owner>/<repo>` with the GitHub
repository path shown in the browser or by `git remote get-url origin`.

### 3. Verify the checksum

On Linux:

```bash
grep "  quecto-${TAG}-${ASSET}.tar.gz$" SHA256SUMS.txt | sha256sum --check -
```

On macOS:

```bash
grep "  quecto-${TAG}-${ASSET}.tar.gz$" SHA256SUMS.txt | shasum -a 256 --check -
```

The command must print `OK`. Do not install the archive if verification fails.
Re-download the asset and `SHA256SUMS.txt`; if verification still fails, report
the release as suspect.

### 4. Install both binaries

Install to a directory on `PATH`. The example below uses `~/.local/bin`:

```bash
mkdir -p "$HOME/.local/bin"
tar -xzf "quecto-${TAG}-${ASSET}.tar.gz"
cp "quecto-${TAG}-${ASSET}/quecto" "$HOME/.local/bin/quecto"
cp "quecto-${TAG}-${ASSET}/quecto-tui" "$HOME/.local/bin/quecto-tui"
chmod 0755 "$HOME/.local/bin/quecto" "$HOME/.local/bin/quecto-tui"
```

Ensure the install directory is on `PATH`:

```bash
case ":$PATH:" in
  *":$HOME/.local/bin:"*) ;;
  *) echo 'Add export PATH="$HOME/.local/bin:$PATH" to your shell profile' ;;
esac
```

Then check the binaries:

```bash
quecto --version
quecto-tui --version
```

`quecto-tui` launches a local harness process by running `quecto`. If
`quecto-tui` reports that `quecto` cannot be found, install both binaries into
the same `PATH` directory or start the harness yourself and connect with
`quecto-tui --socket /path/to/quecto.sock`.

### WSL2 notes for Windows users

No native Windows archive is published. Use WSL2 instead:

1. Install WSL2 and a Linux distribution such as Ubuntu.
2. Open the WSL2 shell.
3. Download the Linux asset from inside WSL2, normally `linux-x86_64`.
4. Install into a Linux path such as `~/.local/bin`, not into `C:\Program Files`
   or another Windows filesystem path.
5. Run `quecto` and `quecto-tui` from the WSL2 terminal.

This keeps Unix-domain socket behavior, process handling, file permissions, and
terminal control on the Linux side where Quecto is built and tested.

## Maintainer runbook

### Before marking a PR for binary release

1. Confirm the PR targets `master`.
2. Confirm the PR is the commit you intend to ship and that normal merge CI is
   green.
3. Confirm crate versions and user-visible docs are correct when the release is
   meant to advertise a version bump. The workspace README has a “Current
   version” line for the harness; package versions live in each package's
   `Cargo.toml`.
4. Add the `release-binaries` label to the PR before merging:

   ```bash
   gh pr edit <PR_NUMBER> --add-label release-binaries
   ```

5. Merge the PR into `master` using the repository's normal merge process.

The automatic workflow run starts from the `pull_request_target` `closed` event
after GitHub records the PR as merged. If the label is absent when the PR is
merged, no automatic binary release is produced.

### Monitor the automatic release

Open **Actions → Binary Release** and inspect the run. A successful run has
these stages:

1. **Validate release request** — verifies that the PR was merged, targeted
   `master`, carried `release-binaries`, resolves the exact merge commit, and
   selects the UTC date tag.
2. **Build linux-x86_64 / linux-arm64 / macos-intel / macos-apple-silicon** —
   checks out the exact merge commit and runs:

   ```bash
   cargo build --release --locked --target <target> \
     -p quecto-agentic-harness \
     -p quecto-tui
   ```

3. **Publish GitHub release** — creates the annotated tag if needed, verifies
   there are four archives, writes `SHA256SUMS.txt`, checks those checksums, and
   creates or updates the GitHub release.

The workflow summary records the PR number, target commit, tag, and published
asset count.

### Manual retry or recovery

Use `workflow_dispatch` when the automatic release failed due to an external or
transient problem, such as runner capacity, network failure, or a partially
created release.

1. Find the merged PR number and merge commit:

   ```bash
   gh pr view <PR_NUMBER> --json number,merged,baseRefName,mergeCommit,labels
   ```

2. Start **Actions → Binary Release → Run workflow** and choose the `master`
   branch in the branch selector.
3. Enter:
   - `pr_number`: the merged PR number that carried `release-binaries`.
   - `target_sha` (optional but recommended): the full 40-character merge commit
     SHA from the PR.

The retry will refuse to run if the PR was not merged, did not target `master`,
did not carry `release-binaries` at merge time, or if `target_sha` does not
exactly match the PR's merge commit. If a release for the intended tag already
exists, the publish step uploads the four archives and `SHA256SUMS.txt` with
`--clobber` so reruns can repair missing or corrupt assets.

### Collision handling and serialization

The workflow's concurrency group is `binary-release`, with
`cancel-in-progress: false`. Do not cancel an in-progress release to start
another one unless you have decided to abandon and manually clean up the first
attempt. Queued release runs are safer because they serialize tag collision
selection and publication.

If two eligible PRs are merged on the same UTC date, the first uses
`vYYYY.MM.DD`; the second sees that tag and uses `vYYYY.MM.DD.1`. Further
same-day releases increment the suffix.

### Cleaning up a failed release

Prefer rerunning the workflow for the same PR first. If manual cleanup is truly
needed:

1. Inspect the tag and release:

   ```bash
   TAG=vYYYY.MM.DD
   git fetch --tags origin
   git rev-list -n 1 "$TAG"
   gh release view "$TAG"
   ```

2. If the tag points at the correct commit but assets are incomplete, rerun the
   workflow_dispatch path for the same PR. The publish job will reuse the tag and
   clobber assets.
3. If a tag was created for the wrong commit, do not move it silently. Delete or
   supersede it only after maintainers agree on the incident response and users
   are told what happened.

### Local reproduction of a target build

From a clean checkout of the intended commit:

```bash
git fetch origin master --tags
git checkout <MERGE_COMMIT_SHA>
rustup target add x86_64-unknown-linux-gnu
cargo build --release --locked --target x86_64-unknown-linux-gnu \
  -p quecto-agentic-harness \
  -p quecto-tui
```

Use the matching Rust target from the platform table for other targets. macOS
binaries should be reproduced on macOS runners or hosts for the matching Apple
architecture; Linux ARM64 releases are built on a Linux ARM64 runner.

## Security notes

- Treat the GitHub release page and `SHA256SUMS.txt` as the source of truth for
  published archives.
- Checksums detect accidental corruption and mismatched downloads; they are not
  a substitute for reviewing the release PR and the workflow run provenance.
- The release workflow intentionally avoids Windows artifacts until the project
  has a tested native Windows support policy.
- The release token has contents write permission only during publication, after
  all platform builds have completed successfully.
