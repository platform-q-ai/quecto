# Quecto Agentic Harness Mono-Repo Migration Plan

## Assumption

The Rust package/crate name remains `quecto`. Only its package directory moves from the repository root into `quecto-agentic-harness/`.

## Goals

- Make the repository a proper Cargo mono-repo with every package in its own root folder.
- Move the current root `quecto` package into `quecto-agentic-harness/`.
- Colocate each package's source, tests, docs, and related package files under the correct workspace package root.
- Move only harness-owned docs under `quecto-agentic-harness/`; docs for API, MCP, runtime manager, TUI, or cross-workspace concerns should live with their owning package or remain at the workspace root when genuinely shared.
- Keep the repository root focused on workspace-level configuration and shared tooling.

## Implementation Steps

1. Create `quecto-agentic-harness/` as the new package root for the current root `quecto` crate.

2. Move package-owned files into `quecto-agentic-harness/`:
   - `src/`
   - `tests/`
   - Package-level docs/config that belong to the harness, including likely candidates such as `README.md`, `workflow-config.json`, and `chain.toml` after confirming their usage.

3. Classify and migrate documentation by owning workspace project:
   - Audit every file currently under root `docs/` before moving it.
   - Move agentic harness docs to `quecto-agentic-harness/docs/`.
   - Move TUI-specific docs to `quecto-tui/docs/` or fold them into `quecto-tui/README.md` if that is the existing package convention.
   - Move API-specific docs to `quecto-api/docs/` or `quecto-api/README.md`.
   - Move MCP-specific docs to `quecto-mcp/docs/` or `quecto-mcp/README.md`.
   - Move runtime-manager-specific docs to `quecto-runtime-manager/docs/` or `quecto-runtime-manager/README.md`.
   - Keep truly workspace-level docs at the repository root, preferably under a root-level `docs/`, only when they describe the whole mono-repo rather than one package.
   - Update all links and embedded-doc references after each doc is moved.

4. Split `Cargo.toml` responsibilities:
   - Keep the root `Cargo.toml` as workspace-only metadata.
   - Create `quecto-agentic-harness/Cargo.toml` from the current root package metadata.
   - Preserve package name `quecto`.
   - Update workspace members to include `quecto-agentic-harness` instead of `.`.

5. Update relative Cargo paths:
   - Change the harness dev-dependency on `quecto-tui` from `path = "quecto-tui"` to `path = "../quecto-tui"`.
   - Check for any additional path dependencies that depended on the package living at the repository root.

6. Update source, test, and documentation path references:
   - Adjust test helpers that read `README.md`, `docs/...`, `src/...`, or other root-relative files.
   - Update documentation links affected by moving `docs/` under `quecto-agentic-harness/`.
   - Update documentation links affected by moving project-specific docs into other workspace package roots.
   - Update `chain.toml` paths according to its final location.

7. Update workspace tooling and CI:
   - Review `.github/workflows/ci.yml`, `tarpaulin.toml`, `deny.toml`, `clippy.toml`, `rustfmt.toml`, and `scripts/` for root-package assumptions.
   - Prefer workspace-level commands where appropriate, such as `cargo test --workspace`.
   - Update package-specific commands to use `-p quecto` or paths under `quecto-agentic-harness/`.

8. Verify the migration:
   - Run `cargo metadata` to confirm workspace/package resolution.
   - Run `cargo test -p quecto --features test-support` or the relevant package-specific test command.
   - Run `cargo test --workspace` if feasible.
   - Search for stale references to moved `src/`, `tests/`, and `docs/` paths and fix any remaining mismatches.
   - Confirm no documentation file was moved into `quecto-agentic-harness/` unless it is owned by the harness package.

## Expected Final Layout

```text
.
├── Cargo.toml
├── Cargo.lock
├── quecto-agentic-harness/
│   ├── Cargo.toml
│   ├── README.md
│   ├── src/
│   ├── tests/
│   └── docs/
├── quecto-api/
│   └── docs/              # if API-owned docs exist
├── quecto-mcp/
│   └── docs/              # if MCP-owned docs exist
├── quecto-runtime-manager/
│   └── docs/              # if runtime-manager-owned docs exist
├── quecto-tui/
│   └── docs/              # if TUI-owned docs exist
└── docs/                  # only genuinely workspace-level docs, if any
```

## Verification Notes

- The crate/package remains addressable as `quecto` in Cargo commands.
- The repository root no longer contains package-owned `src/` or `tests/` directories for the harness crate.
- Root-level `docs/`, if retained, contains only workspace-level documentation rather than package-specific documentation.
- Package-specific documentation is colocated with its owning workspace project.
- Workspace members are all explicit package directories.
