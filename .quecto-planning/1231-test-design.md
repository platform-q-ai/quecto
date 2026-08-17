# Issue 1231 test design

Trace tests to ACs and semantic rows. Reviewer findings resolved by making AC16/check evidence, old-client, collapse, abort, routing, preference, compatible negatives, redacted granularity, and docs criteria falsifiable.

## Provider normalization (AC2, AC3, AC4, AC14, AC16)
- Anthropic fixture tests: visible thinking deltas produce display-safe thinking; multiple redacted blocks interleaved with visible thinking produce ordered per-block `[Redacted thinking]` placeholders; recovery-visible order matches live order; signature/redacted payloads are absent from visible output.
- OpenAI Responses fixture tests: reasoning summary deltas produce thinking; encrypted continuity remains internal and never appears in visible text.
- OpenAI-compatible fixture tests: exact supported string fields (`reasoning_content`, `reasoning`, `thinking`) produce thinking; object/map `reasoning`, `metadata.reasoning_content`, content-only pseudo-reasoning, non-string reasoning fields, and ambiguous/duplicate answer-like fields fail closed.

## Application/protocol (AC1, AC5, AC11, AC13, AC16)
- Agent-loop/progress tests: provider thinking becomes a distinct progress event; tokens/tool events/done and pre-call Thinking spinner event remain stable.
- UDS DTO tests: additive `thinking` event serializes/parses distinctly from `token`; actual old-client DTO/parser behavior (or an exact strict/tolerant compatibility fixture matching the prior parser shape) ignores/skips the unknown `thinking` event and still handles later token/tool/agent_end events.
- Non-interactive regression: answer-only stdout by default.

## Persistence/recovery (AC6, AC7, AC14, AC16)
- Message view tests: `visibleThinking` is additive and display-safe; private thinking signatures/encrypted/redacted data omitted.
- Session save/reload tests: text-only and tool-using turns retain visible thinking.
- Tool-loop order test: pre-tool and post-tool thinking recover in order around tool boundaries, remain associated with the correct assistant/provider-call segment, and are not merged across tool boundaries.
- Bounded/collapsed history test/inspection: inspect recovered page entries, collapsed stubs, spill metadata, and session JSON; visible thinking may appear only in display-safe fields when the message itself is returned, omitted from collapsed stubs when existing budget behavior omits message detail, and private replay fields (`signature`, `encrypted_content`, redacted blobs) are forbidden in all user-facing DTO/stub/spill labels.
- Abort/error stream safety: if a stream errors/cancels before final assistant commit, recovery either omits the partial assistant message or includes only the display-safe partial thinking already emitted live; assert the chosen repository behavior concretely and assert private fields are absent either way.

## TUI (AC8, AC9, AC10, AC12, AC16)
- Live event rendering/routing: thinking appears as separate labelled/styled section, answer tokens remain separate; child/subagent live thinking routes to master/focused-child views consistently with live tokens and is grouped only within the correct run.
- Recovered rendering: message `visibleThinking` renders in master and focused/child/subagent views.
- Toggle: non-conflicting keybinding/command hides live and recovered thinking as `Thinking...`, restores text, and does not mutate stored DTO/session data.
- Preference persistence: hidden preference survives TUI restart; after restart recovered messages render hidden while API/message recovery still returns full display-safe thinking.
- Effort controls/footer existing tests remain green.

## Docs/checks (AC15, AC16)
- UDS protocol docs and capability matrix must explicitly show the additive live thinking event wire shape, recovered message visible-thinking shape, unknown-event compatibility, and unchanged token/tool/turn/agent_end semantics.
- User/TUI docs must explicitly state default visible TUI rendering, hide/show key/setting, display-only persistence of the preference, answer-only default for non-interactive output, and non-leakage of encrypted/signature/redacted/private provider fields.

## AC16 verification commands and pass criteria
- Provider fixture tests pass for Anthropic, OpenAI Responses, and OpenAI-compatible positive/negative/leakage cases.
- Agent-loop/protocol tests pass for live thinking events, old-client compatibility, stable tokens/tools/turn/agent_end/spinner, and non-interactive answer-only output.
- Persistence/recovery tests pass for text-only/tool turns, reload, bounded/collapsed safety, abort/error safety, and private-field non-leakage.
- TUI tests pass for live/recovered rendering, master/focused-child/subagent routing, hide/show, and remembered preference.
- Docs contain the explicit review criteria above.
- `cargo fmt --check`, targeted `cargo test` commands for changed crates, and required hook/pre-push checks pass without `--no-verify`.
