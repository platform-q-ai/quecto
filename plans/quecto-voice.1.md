<!-- quecto-plan schema=1 -->
# Plan 1 of quecto-voice

| Field | Value |
|---|---|
| Task | quecto-voice |
| Version | 1 |
| State | draft |
| Proposed by | quecto pocket (`pocket@quecto.invalid`) |
| Created | 2026-10-16T06:43:09Z |
| Updated | 2026-10-16T06:43:09Z |

## Approach
Add an opt-in, genuinely full-duplex voice frontend while keeping Quecto's existing harness authoritative for reasoning, tools, permissions and task state. Source review found OpenAI's Introducing GPT-Live (2026-07-08), GPT-Live-1 API launch (2026-09-10), and current Live/client-delegation and Decisions-voice docs; 'Live 1.1' and the intended Decisions blog remain unverified and must be confirmed rather than guessed. Proposed first surface is local TUI voice; Pocket/mobile, telephony and native Decisions execution are separate scope decisions. Put provider-neutral lifecycle/context contracts in application-owned ports, OpenAI/media adapters outside them, and keep audio off the existing durable JSON conversation lane. Each implementation item is a small, independently mergeable opt-in PR: a fresh red-test swarm commits focused compiling tests that fail for intended missing behavior and hands off a SHA-256-verified git bundle without pushing; a fresh delivery swarm preserves that first commit and pushes red+green together. Escalate already-green tests or an unrunnable baseline. Run bounded targeted tests and applicable lint/architecture/docs/contracts locally, full suites in CI; use PR-first fresh independent review, separate remediation and current-head re-verification. Keep process evidence external, resolve review threads, and require passing CI with merge-requested before merge. Source notes: /home/shane-pocket/.local/state/quecto-pocket/files/fbea48b4-13aa-4006-a3d1-7eb358a7322c/out/quecto-voice-sources.md.

## Task items
| Task item | Title |
|---|---|
| voice-contract | Confirm sources, first client and voice architecture contract |
| voice-core | Introduce opt-in provider-neutral voice lifecycle and bounded control |
| voice-live-adapter | Implement the documented OpenAI Live transport adapter |
| voice-delegation | Bridge Live delegation into existing Quecto admission and results |
| voice-audio-client | Add optional local full-duplex capture and playback |
| voice-tui | Expose voice controls, status and transcript in the TUI |
| voice-release | Verify conversational behavior, privacy, budgets and rollout |

## Task item dependencies
| Task item | Depends on |
|---|---|
| voice-core | voice-contract |
| voice-live-adapter | voice-core |
| voice-delegation | voice-live-adapter |
| voice-audio-client | voice-delegation |
| voice-tui | voice-audio-client |
| voice-release | voice-tui |

## Criteria
| Task item | Criterion |
|---|---|
| voice-contract | Confirm what 'decision voice and live 1.1' refers to; record exact official article/docs URLs, dates, supported model IDs, account availability, auth, transport, media and delegation events. Do not advertise an unverified model/version. |
| voice-contract | Approve initial client/platform scope: proposed local TUI, with remote Pocket/mobile and telephony excluded unless explicitly added. Document device/echo-handling and headless behavior. |
| voice-contract | Specify client delegation as baseline: Quecto owns tools/permissions; assess Decisions for optional allowlisted controls without assuming native Decisions and client delegation can coexist in one session. |
| voice-contract | Map existing application/ports.rs, application/agent_loop.rs, interface/cli/protocol_commands.rs, uds_prompt_admission.rs, TUI protocol/client.rs and conversation controllers to new seams. Document states, transcript ownership, cancellation meanings, retention and measurable overlap/latency acceptance; no implementation in this item. |
| voice-core | Add application-owned voice ports/use cases and explicit session identity/state, clock and transcript/control DTOs; infrastructure implements ports, with architecture gates enforcing inward dependencies. |
| voice-core | Tests cover start/stop/mute, invalid transitions, session switch, duplicate/late events, queue exhaustion and deterministic teardown; affirmative allowlist guards reject invalid identities/events before side effects. |
| voice-core | Disabled/unavailable voice opens no microphone or provider connection and leaves existing text-only CLI/TUI behavior unchanged. All queues, waits and session limits are explicit and bounded. |
| voice-live-adapter | Create the confirmed Live session with client delegation and server-side credentials; validate documented model/voice/media settings and redact credentials from diagnostics. |
| voice-live-adapter | Fixture/fake-server tests establish independent concurrent input/output audio and control handling, session-start readiness, transcripts and delegation metadata, malformed/oversized events, authentication/rate-limit failures and orderly close. |
| voice-live-adapter | Backpressure, disconnect and reconnect policy are bounded; reconnect never silently replays non-idempotent work. Keep raw audio out of existing UDS history/event payloads and persistence by default; no chained STT/LLM/TTS fallback marketed as full duplex. |
| voice-delegation | Build backend requests from correlated transcript revisions and application/session state: delegation metadata is not task text. Tests cover partial/final transcripts, corrections, duplicate notifications and out-of-order events without duplicate execution. |
| voice-delegation | Reuse normal prompt/steer/follow_up admission, request IDs, tool policies and confirmations. Distinguish acceptance receipts from completed work and retain existing busy/refusal semantics; voice cannot grant itself tool authority. |
| voice-delegation | Return only verified, redacted user-facing results with the matching session/delegation identity. Suppress stale/cancelled results and hidden reasoning; document transcript/history ownership to prevent double appends. |
| voice-delegation | Playback interruption does not implicitly abort backend work; explicit admitted cancellation remains separate. Targeted tests cover long-running tools while conversation continues and explicit steer/follow-up/abort choices. |
| voice-audio-client | Implement optional audio-device adapters for the approved platforms; keep capture live while playback runs, with negotiated format/resampling and an explicit echo/headphone policy. |
| voice-audio-client | Deterministic audio fakes prove overlapping capture/playback, immediate stale-playback flush, mute, device removal, underrun/overrun, slow consumers and cleanup; unsafe concurrency probes have hard SIGKILL/_exit timeouts. |
| voice-audio-client | Optional dependencies and no-device/headless builds remain usable. Device/permission errors fall back visibly to text without retry storms or leaking active microphone handles. |
| voice-tui | Add explicit opt-in start, mute, stop-playback and disconnect controls with visible listening/speaking/backend-working/error state; typed interaction and backend cancellation remain independently usable. |
| voice-tui | Extend protocol/controller boundaries only as required, preserving framed UDS caps, bounded writer fairness and compatibility with agents/clients lacking voice capability. |
| voice-tui | Tests cover keyboard/control routing, session switching, disconnect while busy and partial/final transcript rendering. Final transcript/history follows the approved retention policy; raw audio and hidden reasoning are not persisted or displayed. |
| voice-release | Add scripted end-to-end BDD scenarios for overlapping speech, pauses/backchannels, corrections, long-running work, interruptions, duplicate/stale results, transport/device failure and text fallback. Establish full duplex from timestamped evidence, not alternating-turn demonstrations. |
| voice-release | Enforce configured session-duration/inactivity limits and expose voice duration separately from backend usage; verify default no-audio-retention and secret/context-redaction behavior with focused tests. |
| voice-release | Publish intentional user/configuration documentation for enablement, supported OS/devices, credentials, privacy, limitations, cancellation and troubleshooting; record scoped real-device smoke and latency/echo evidence for non-testable criteria. |
| voice-release | Each prior PR remains green and independently usable behind opt-in controls; current-head independent review and required CI pass, all actionable threads resolve, and no process artifacts enter code diffs. |
