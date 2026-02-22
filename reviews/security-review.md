# Security Review

Date: 2026-02-22
Scope: Full repository review (not PR diff)

## Executive Summary

- The codebase has solid defensive building blocks (path canonicalization, Telegram API base validation, credential file permission hardening), but there are several high-impact gaps in how dangerous capabilities are exposed.
- A plaintext OpenAI key is present in `.env`, which is an immediate secret-management incident requiring rotation.
- Command execution is currently shell-based (`sh -c`) with denylist-only filtering and no enforced command allowlist in runtime wiring, enabling broad command execution if an agent run is influenced by untrusted input.
- The outbound messaging permission model is too permissive: the `message` tool can route to arbitrary Telegram targets instead of being constrained to the current conversation.

## Findings

| ID | Severity | Title | Evidence | Risk | Recommendation |
|---|---|---|---|---|---|
| SEC-001 | Critical | Plaintext API key in workspace `.env` | `.env:1` | A live provider secret is stored in plaintext in the repository workspace; accidental disclosure/commit or local compromise can immediately expose provider access. | Revoke/rotate this key immediately; remove the secret from local repo state; enforce secret scanning in CI/pre-commit and use environment/credential store only. |
| SEC-002 | High | Exec tool allows broad shell execution with denylist-only controls | `src/infrastructure/tools/exec.rs:99`, `src/infrastructure/tools/exec.rs:100`, `src/infrastructure/tools/exec.rs:101`, `src/infrastructure/security/sandbox.rs:59`, `src/infrastructure/security/sandbox.rs:119`, `src/infrastructure/security/sandbox.rs:132`, `src/infrastructure/tools/registry.rs:44`, `src/infrastructure/tools/registry.rs:49` | Commands are executed via `sh -c`, so shell parsing/injection primitives are available; runtime uses default `command_allowlist: None`, leaving only substring denylist checks that are bypass-prone and do not confine command effects to workspace. | Replace `sh -c` with direct binary+arg execution; enforce a strict command allowlist by default in production wiring; add per-command argument validation and deny absolute/out-of-workspace file arguments. |
| SEC-003 | High | Outbound messaging tool can target arbitrary chats (permission model gap) | `src/infrastructure/tools/message.rs:53`, `src/infrastructure/tools/message.rs:57`, `src/infrastructure/tools/message.rs:63`, `src/interface/gateway/mod.rs:280`, `src/interface/gateway/services.rs:104`, `src/interface/gateway/services.rs:113` | The agent can send messages to arbitrary `telegram:<chat_id>` targets, enabling cross-chat spam/data exfiltration if prompted or compromised. | Bind `message` tool to conversation-scoped default target in gateway and reject caller-supplied target overrides unless explicitly authorized by policy. |
| SEC-004 | Medium | Provider `api_base` is not security-validated before sending API keys | `src/infrastructure/providers/mod.rs:11`, `src/infrastructure/providers/openai.rs:22`, `src/infrastructure/providers/openai.rs:154`, `src/infrastructure/providers/anthropic.rs:22`, `src/infrastructure/providers/anthropic.rs:181` | A malicious/misconfigured `api_base` can redirect bearer keys to attacker-controlled endpoints (including plaintext HTTP), causing credential exfiltration and SSRF-like behavior. | Add strict URL validation (https-only by default, expected host allowlist, no credentials/query/fragment), with explicit opt-in test override env flags as done for Telegram. |
| SEC-005 | Medium | Upstream error bodies are propagated to users/log paths | `src/infrastructure/providers/openai.rs:163`, `src/infrastructure/providers/openai.rs:282`, `src/infrastructure/providers/anthropic.rs:191`, `src/infrastructure/providers/anthropic.rs:320`, `src/interface/gateway/services.rs:91`, `src/interface/gateway/services.rs:92`, `src/interface/cli/agent.rs:341` | Raw provider response bodies may include sensitive diagnostic/request context and are returned to end users (`Error: ...`) and logs, increasing data exposure risk. | Sanitize provider errors before surfacing; return structured generic messages to users while logging minimal redacted diagnostics server-side. |
| SEC-006 | Low | Filesystem tools have TOCTOU window after path validation | `src/infrastructure/tools/filesystem.rs:67`, `src/infrastructure/tools/filesystem.rs:124`, `src/infrastructure/tools/filesystem.rs:129`, `src/infrastructure/tools/filesystem.rs:254`, `src/infrastructure/security/sandbox.rs:81`, `src/infrastructure/security/sandbox.rs:102` | Path is validated first, then file operations occur later; a local attacker with filesystem race capability could swap symlinks between check/use to escape intended boundaries. | Use race-resistant open patterns (`openat`-style with `O_NOFOLLOW`, re-validate resolved parent inode at open time) for write/append/edit paths. |

## Positive Observations

- Path boundary enforcement is implemented with canonicalization and symlink-aware checks in `src/infrastructure/security/sandbox.rs:64` and tested for symlink escape cases (`src/infrastructure/security/sandbox.rs:380`, `src/infrastructure/security/sandbox.rs:418`).
- Telegram endpoint hardening is strong: strict API base validation rejects non-HTTPS/non-default hosts unless explicitly overridden for testing (`src/infrastructure/channels/telegram.rs:88`, `src/infrastructure/channels/telegram.rs:104`).
- Credential storage uses restrictive Unix permissions and re-enforces `0600` on every write (`src/infrastructure/auth/credential_store.rs:130`, `src/infrastructure/auth/credential_store.rs:150`).
- Voice/file ingestion applies size limits before and during download to reduce resource-exhaustion risk (`src/interface/gateway/telegram.rs:12`, `src/infrastructure/channels/telegram.rs:260`, `src/infrastructure/persistence/skill_loader.rs:13`).

## Quick Wins

1. Rotate the leaked OpenAI key from `.env` immediately, replace with a new key, and add automated secret scanning in CI + pre-commit.
2. Lock down `exec`: remove `sh -c`, require command allowlist at runtime, and block unsafe argument patterns by default.
3. Constrain `message` tool to current-session targets only (no arbitrary `target` unless an explicit admin policy grants it).
