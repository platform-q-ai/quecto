# Issue 1231 semantic risk matrix

## Dimensions
Provider source: Anthropic thinking vs Anthropic redacted vs OpenAI Responses summary vs explicit OpenAI-compatible reasoning field vs unsupported/no thinking. Stream shape: live deltas, interleaved with tokens/tools, text-only final, tool-using turn. Consumer: UDS old/new, quecto-api, TUI master, TUI focused child/subagent, message recovery. State: fresh live turn, post-turn same process, after session restart, bounded/collapsed history. Visibility pref: default visible, hidden placeholder, restored, restart remembered. Safety: encrypted/signature/redacted/private fields present. Out of scope dimensions: effort vocabulary changes, provider live calls, summarization, per-block collapse.

| AC/invariant | Dimensions/classes | Representative/high-risk cases | Expected observable outcome | Observation/correlation | Planned evidence |
|---|---|---|---|---|---|
| AC1/5/13 live distinct events | thinking deltas, tokens, spinner, tools | thinking before first token; thinking interleaved with tool start; old client sees unknown event | `thinking` event distinct from `token`; pre-call status unchanged; tools unchanged; unknown events ignored | event stream order and fields | BDD/unit protocol tests |
| AC2 Anthropic | plain thinking, redacted thinking, signatures | redacted block includes encrypted/signature-like payload | visible thinking text or `[Redacted thinking]`; no signature/encrypted blob | provider parse output/progress | fixture parser tests + leakage asserts |
| AC3 OpenAI Responses | summary delta/done, encrypted continuity | summary streams while encrypted content included | summary visible as thinking; encrypted stays internal | parser events and message recovery | fixture parser tests |
| AC4 compatible | explicit `reasoning_content`/`reasoning` fixtures vs arbitrary metadata | unknown private metadata contains plausible text | only supported fields parsed; otherwise no thinking | parser output empty for unsupported | fixture tests |
| AC6/7 persistence/recovery | text-only, tool turn, reload, get_message(s), collapsed/bounded | text-only finalization after thinking only then answer; history budget boundary | message view includes additive display-safe thinking; stored private replay retained internally | JSON DTOs/session reload | persistence/recovery tests |
| AC8 TUI render | master, focused child, subagent, recovered/live | child thinking while focused; master transcript groups thinking | distinct styled/labelled section not merged into answer | render snapshots/probes | TUI BDD/unit tests |
| AC9/10 pref | visible default, hidden, shown, restart | hidden live then recovered after restart | placeholder `Thinking...` when hidden; text restored; preference stored outside messages | render + config file/state | TUI tests |
| AC11 | one-shot/print vs TUI | reasoning-only early stream in noninteractive mode | stdout remains answer-only by default | process output | regression test/inspection |
| AC12 | effort controls | /effort, footer, set_effort | unchanged behaviour and labels | existing tests remain green | existing tests + no code path changes |
| AC14 safety | unsupported/no private leakage | encrypted/signature/redacted blob present in provider frames/session | no user-visible leakage, fail closed | grep/assert rendered/recovered text | leakage tests |
| AC15 docs | protocol/recovery/TUI | docs omit wire shape or non-leakage | docs describe additive events/recovered fields/hide-show/noninteractive | doc diff | docs update review |

Additional counterexample-driven rows:

| AC/invariant | Dimensions/classes | Representative/high-risk cases | Expected observable outcome | Observation/correlation | Planned evidence |
|---|---|---|---|---|---|
| AC6/7/14 abort safety | cancel/error after partial thinking; partial private frame | thinking delta then stream error before final answer | explicit chosen behavior: no private leakage; if partial assistant recovery exists it contains only display-safe thinking/placeholder, otherwise omitted consistently | live stream + session/recovery after error | abort/error stream test or justified inspection of non-persistence |
| AC6/8 tool-loop ordering | multiple provider calls in one user turn | pre-tool thinking, tool call/result, post-tool thinking, final answer | thinking is associated with the correct assistant segment/order; not grouped across tool boundary | live events and recovered ordered DTO/render | agent-loop/persistence test |
| AC5 strict old clients | unknown event with strict enum/parser | old parser receives thinking then later token/end | existing clients demonstrably ignore unknowns or new event is delivered through extensible/gated path | compatibility parser test using old/new DTO behavior | old-client compatibility test |
| AC6/7/14 collapse/private divergence | collapsed/spilled message with visible thinking and replay-only private fields | history budget boundary with redacted/signature/encrypted data | user DTO/stub only display-safe or safe omission; private replay never in stubs; internal replay retention separate | recovered pages/stubs + provider replay inspection | bounded/collapsed history test |
| AC9/10 API independence | hidden TUI preference plus recovery | hidden pref then get_messages and TUI render | API returns full display-safe thinking; TUI hides at render time only | DTO vs render output | TUI/API preference test |
| AC4/14 exact compatible paths | ambiguous object/metadata reasoning | object/map reasoning, metadata reasoning_content, content-only, duplicate answer | only exact supported string fields emit thinking; unsupported shapes fail closed | parser fixtures | compatible parser negative tests |
| AC2 redacted order | multiple redacted blocks interleaved | visible A, redacted1, visible B, redacted2 | placeholder granularity is per redacted block/contiguous region, ordered, payload-free; recovery matches live | event/recovery sequence | Anthropic fixture test |

High-risk interactions map to tests: provider private payload + recovery/TUI, text-only finalization + reload, child routing + hidden pref, old client + new event, token/spinner stability + thinking, abort safety, multi-call tool-loop ordering, collapse/private divergence, API independence from TUI preference, exact compatible field paths, redacted ordering.

## Semantic review disposition
- Accepted high: abort/incomplete stream safety; added explicit row and planned evidence.
- Accepted high: multiple model calls/tool-loop ordering; added explicit row and planned evidence.
- Accepted high: strict old-client schema compatibility; added explicit row and planned evidence.
- Accepted medium: bounded/collapsed private/display divergence; added explicit row and planned evidence.
- Accepted medium: hide preference must not filter API recovery; added explicit row and planned evidence.
- Accepted medium: exact OpenAI-compatible field paths/types; added explicit row and planned evidence.
- Accepted medium: redacted placeholder ordering/granularity; added explicit row and planned evidence.
