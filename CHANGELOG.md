# Changelog

## 0.15.0 (2026-03-03)

### Added / Updated
- **RPC mode for CLI agents**: `quecto agent --mode rpc` now exposes a JSON-lines protocol for long-lived headless operation and external tooling.
- **Cancel support**: `CancelFlag` is plumbed through request paths to support aborting in-flight calls when possible.
- **User message content blocks**: user messages now support structured content (including inline image blocks) with provider capability filtering.
- **Cross-provider message normalization**: conversation messages are normalized between providers for consistent tool-call handling.
- **Anthropic streaming improvements**: true incremental SSE streaming is implemented with richer stream event handling.
- **Anthropic provider enhancements**:
  - Per-call cost tracking and pricing metadata.
  - Extended-thinking mode support.
  - Tool batching, `tool_choice`, SSE usage accounting, and stop-reason reporting.
- **Extensions and operational tooling updates**:
  - Added agent-manager dashboard and heartbeat `.pi` extension.
  - Dashboard + heartbeat integration updates.
  - Pi extension refactor work across several components (`Cow<str>`, `ImageBlock`, spawn-blocking grep path, etc.).

### Documentation
- Updated user-facing docs to describe the new `--mode rpc` headless mode and abort-friendly RPC behavior.
- Reviewed recent API/behavior changes from the last 10 PRs and aligned docs/versioning for release notes.

### Source PRs reviewed
- #233, #182, #188, #184, #181, #229, #185, #175, #216, #228

## 0.14.0

See `0.14.0` baseline release that aligned docs and architecture references.
