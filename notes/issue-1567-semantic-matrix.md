# Issue #1567 semantic matrix — full AC completion

Frozen contract decisions after semantic review:
- `get_session_stats` must serialize normalized cost. Use explicit `costMicroUsd` integer to avoid float/rounding ambiguity; TUI can render dollars if desired.
- Cache-hit ratio is a shared stats field, computed once in application/domain from normalized totals and serialized as `cacheHitRatio` (`Option<f64>`, 0.0-1.0). Surfaces consume this field; they do not recompute provider-specific ratios.
- Cumulative cache-hit denominator: `cache_read / (input + cache_read + cache_write)` where `input` is normalized full-price input. Absent all denominator tokens => `None`; denominator > 0 with zero reads => `Some(0.0)`.
- Cost calculation must be centralized at the shared accounting boundary. Provider adapters may parse model/provider identity and usage but must not own surface/reporting math.
- TUI typed session stats must preserve input/output/cacheRead/cacheWrite/total/context/costMicroUsd/cacheHitRatio so `/session`, status, and detail surfaces consume one shared DTO.

Tempting but irrelevant: provider request options, auth, persistence migrations, billing dashboard.

| AC/invariant | Dimensions/equivalence classes | Representative/high-risk cases | Expected observable outcome | Observation | Planned evidence |
|---|---|---|---|---|---|
| Providers map to one normalized structure | Anthropic separated read/write; OpenAI Chat/Responses cached subset; no usage | Equivalent Anthropic input=70 read=30 vs OpenAI provider input=100 cached=30 | Shared stats see input=70, cache_read=30, context=100 regardless provider | stats/log/TUI values match fixture | existing parser tests + shared stats tests |
| Shared cache-hit ratio | absent/zero/read/write; denominator zero/nonzero; per-call vs cumulative | read=30,input=70,write=0 => 0.30; write-only input=0 read=0 write=50 => 0.0; no tokens => None | One shared helper computes `cacheHitRatio`; adapters/TUI do not recompute | get_session_stats/log/TUI expose same value | application tests + interface/TUI tests + grep inspection |
| Cost accounting shared | known pricing/missing pricing; cache read/write costs; provider equivalent inputs | OpenAI and Anthropic equivalent usage cost same via normalized usage; cost fixture 1234 micro USD | Cost attached/aggregated in shared path and serialized as `costMicroUsd`; missing pricing preserves tokens and omits/zeros cost per DTO contract | stats/log/TUI include same cost micro value | application cost/stats tests, provider grep |
| get_session_stats primary API | zero sessions; one result; multi-call; missing cost | after recording AgentResult fixture input=70 output=20 read=30 write=5 context=105 cost=1234 | API returns normalized tokens, cache, context, costMicroUsd, cacheHitRatio | tool/protocol response | get_session_stats tests |
| Structured logs | normal result; no usage; cost absent | result with usage emits token/cache/context/costMicroUsd/cacheHitRatio fields exactly once from session stats | dev observes structured fields in logs | tracing/log capture test | log test |
| TUI /session | stats with/without cache/cost/ratio; large numbers | session stats fixture input=70 output=20 read=30 write=5 cost=1234 ratio=30/105 | `/session` displays normalized token/cache/cost/ratio/context from typed session stats | rendered text/state | TUI session test |
| TUI status/detail shared stats | status/detail after stats response; provider identity irrelevant | Inject session-stats DTO only; no provider payload | UI shows same values as `/session` and does not branch by provider wire fields | render/state tests + grep | TUI tests/inspection |
| Architecture ownership | provider wire names outside infrastructure | grep domain/application/interface/TUI for native wire fields | no native wire JSON names outside adapters/tests/docs/notes | grep | architecture check |
| Compatibility | old prompt/completion only; no usage; existing Anthropic/Responses | no panic; previous visible values preserved except newly exposed normalized stats fields | existing tests pass | regression suite | targeted tests |
| Cross-surface consistency | same AgentResult drives stats/log/API/TUI | fixture input=70 output=20 read=30 write=5 context=105 costMicroUsd=1234 | all surfaces report identical normalized stats and ratio | compare outputs | acceptance coverage |
