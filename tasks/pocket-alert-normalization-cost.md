<!-- quecto-task schema=1 -->
# Measure and bound Legacy Question notification normalization work

| Field | Value |
|---|---|
| Id | pocket-alert-normalization-cost |
| Kind | task |
| Status | draft |
| Parent | issue-2708 |
| Created | 2026-10-24T21:35:47Z |
| Updated | 2026-10-24T21:35:47Z |

## Description
Follow-up in platform-q-ai/quecto-pocket, discovered during PR141 independent R4 review at7d29db39b9ff79f25f8552fc3c54952882f19186: AlertText.kt23 plausibly sanitizes full Legacy Question raw input and performs duplicate bounded Markdown stripping on the background alert dispatcher. No measured latency/OOM or correctness failure was established. Independently measure realistic/max-supported inputs; if beneficial, implement a bounded NORMALIZED-prefix single-strip path preserving unsafe-control/bidi/invisible fallback, compression, contextual RGI flags/RTL/combining/emoji, privacy/non-Question Done behavior, and exact raw storage/export/CopyShare/rich full reading. Tests first, meaningful counters/bounds, no speculative performance claim or unrelated cache/lifecycle rewrite. This is deliberately separate from PR141's acquisition-policy fix and requires its own accepted plan/authorization; no live relay/state/units/admission changes, no APK/deploy. [Source review thread](https://github.com/platform-q-ai/quecto-pocket/pull/141#discussion_r4368584503). Parent checkpoint6389824851 accepts current background cost as disclosed tradeoff for PR141.
