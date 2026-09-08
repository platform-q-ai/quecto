# P2 runtime ingress proposal (test-only RED design)

No change to `AgentRuntimeInputs` or existing `Config` struct literals. Keep
`AgentProviderRuntimeFactory` / `compose_agent_provider` admission-disabled.
No environment variable, CLI config or production default enables this path.

Minimal additional infrastructure API (names provisional; parent coordinates
with attempt-gate owner):

```rust
pub struct AdmissionRuntimeContext { /* immutable effective proposal + bound gates */ }
pub struct AdmissionRuntimeProposal {
    pub policy: AdmissionConfig,
    // Explicit router/provider slot -> opaque endpoint/account alias.
    // AdmissionConfig.aliases resolves the alias to the shared quota group.
    pub bindings: BTreeMap<String, String>,
}
pub struct AdmissionRuntimeCandidate<'a> {
    pub provider_config: &'a Config,
    pub admission: Option<&'a AdmissionRuntimeProposal>,
}
pub struct AdmissionProviderRuntimeFactory { /* stable context */ }
impl AdmissionRuntimeContext {
    pub fn new(
        proposal: AdmissionRuntimeProposal,
        // Existing application::inference_attempt::AttemptAdmission is already
        // scope/alias-bound; acquire() deliberately accepts no caller identity.
        gates_by_alias: BTreeMap<String, Arc<dyn AttemptAdmission>>,
    ) -> Result<Self, String>;
}
impl AdmissionProviderRuntimeFactory {
    pub fn new(context: Arc<AdmissionRuntimeContext>) -> Self;
}
// Implement existing generic ProviderRuntimeFactory for Candidate and unchanged
// AgentRuntimeInputs. ComposeProviderRuntimeUseCase already preserves BOTH stores
// and exposes RuntimeCompositionError.retained when factory rejects.
```

A helper `compose_agent_provider_with_admission(config, inputs, context,
proposal)` can implement the inner composition. The stable factory above is
needed only to drive the existing transactional publication use case; no new
publication store or config mutability is needed.

Restart-only validation must compare effective VALUES, not Arc identity, Debug
output, credentials, display names or model IDs. P1 types currently lack
PartialEq: either derive `PartialEq, Eq` on `GroupPolicy` / `AdmissionConfig` or
implement one private exhaustive field comparison. Compare all policy fields,
max_scopes, terminal_capacity, aliases AND bindings. Reject Some->None and changed
C/reserve/interval/queue/deadline/cooldown/map; freshly allocated equal input is a
successful no-op. Validate every constructed provider slot has a binding whose
alias exists in policy before publishing or sending. Ordinary credential/model
rebuild uses the original context unchanged. Do not persist a candidate proposal
until success, and never migrate outstanding attempts into a fresh budget.

All factory closures need the same resolved binding/context, including private
`registry_provider_factory`, built-in Anthropic OAuth, and caller-supplied OpenAI
OAuth rebuild. The last existing closure returns a provider without any gate;
simply wrapping that result outside LlmProvider cannot enforce leaf attempts.
Supply an additional admission-aware rebuild closure through the alternate
factory/context API, or reconstruct the Codex-aware leaf with context in the
alternate composition path. Do not change the existing ProviderFactory signature
or add a mandatory field to AgentRuntimeInputs. Binding must be captured before
refresh, not derived from the replacement token/account claims.

## Executable test seam

`tests/inference_admission_runtime.rs` defines `CurrentRuntimeIngress`, a
forward-only RED bridge to the real `AgentProviderRuntimeFactory`: it deliberately
does NOT implement alias validation, admission, or reload rejection in test code.
The bridge's stable authority, candidate policy and bindings are explicit inputs
for parent integration. Replace that delegation with the alternate production API
above and use the sibling's gate adapter over this same authority/clock. Do not
make the bridge itself enforce the expected behavior (that tests a fake).

Tests invoke real composed public LlmProvider methods and real loopback HTTP,
inspect the public AdmissionClient/Dispatcher state and exact requests/headers/
models, and compose through the real transactional publication use case. Seeded
active/cooldown attempts are authority fixtures, not claimed provider sends.
Queued calls are polled directly to the admission barrier, not checked after a
wall-clock sleep. This requires gate acquire to enqueue on its first poll; if the
final gate runs a separate scheduler, replace that helper with an explicit enqueue
ack barrier. Safety deadlines only bound positive I/O. No ignored tests, no
pointer-only assertions, and no production activation.


## Refined executable RED evidence and precise limitations

Command: `cargo test -p quecto-agentic-harness --test inference_admission_runtime`.
Log: `/tmp/p2-runtime-split-red.log`. Result: **15 compiled/executed, 1 passed,
14 failed, 0 ignored**. No production files changed by this task. Parent's
`poll_once` `?Sized` correction and no-proxy HTTP client are retained. Test target
was rustfmt-formatted. Parent separately recorded a wrong-credential expectation
mutation for the disabled characterization, failing then restored; this task does
not claim additional mutation evidence from that run.

The former restart loop is now twelve separately named executable tests:
capacity, reserve, interval, queue capacity, queue timeout, attempt timeout, max
cooldown, max scopes, terminal capacity, alias map, binding map, disable. Each
mutation is valid on its own (not a generic invalid-config test). Each currently
fails at its own `expect_err`: the disabled forwarding factory incorrectly
publishes candidate generation 3. This is behavioral RED for every requested
rejection variant, not just the first capacity case.

|Assertion family|Current observation|Not yet proved / subsequent evidence owed|
|---|---|---|
|Disabled public chat, credential/model rebuild|PASS: exactly two raw requests, stripped model-v1/model-v2, old/rotated Bearer values, runtime-ok content|No enabled admission claim; parent has credential expectation mutation evidence|
|Rebuilt alias shares retained budget|RED at exact shared queued count: got 0, expected 1 after first poll|First Poll::Pending alone proves only async pending, not admission; subsequent zero-wire assertion currently masked|
|Distinct explicit group with identical endpoint/credential|Executable later in rebuild test, currently masked by shared queued-count RED|Must observe exact gamma wire request and independent completion under shared cooldown after integration|
|Credential/model rebuild exact boundary|Executable polls at 90 and completes at 91, then old runtime at 92; currently masked|Need enabled wire count/models/credentials and retained active=1; changed min interval/new-context mutations owed|
|Unknown explicit alias|RED: candidate composition unexpectedly succeeds|Zero-wire assertion currently masked by rejection assertion; separately omit a provider binding in integration coverage; test currently changes a live candidate, not invalid initial context construction|
|No-op allocated by value under active+cooldown|Each reload fixture succeeds and increments generation while seeded active=1/cooldown=91|This is acceptance evidence only; does not prove retained leaf enforcement. Later blocked-request oracle is masked by rejection RED|
|All 12 restart-only mutations|Each independently RED: candidate publishes instead of returning error|Exact restart error text, retained error snapshot generation, both stores unchanged are masked until rejection exists|
|Retained active/cooldown after failed reload|Executable active=1/cooldown=91 checks, currently masked|An untouched fixture authority alone cannot prove runtime retained it; must also pass subsequent real beta queued-group and wire checks|
|Retained live provider mapping and credential|Executable beta queued in shared not separate, blocked at 90, exact wire at 91 with live-secret and retained-model; currently masked|Need independent mutations after integration: publish rejected candidate, replace authority, apply candidate binding, clear active, clear cooldown, change credential|

These tests intentionally do **not** claim full P2-11 completion or per-assertion
RED/green mutation evidence. Nor do they test a real 401 OAuth factory callback:
current executable rebuilds the real runtime factory with rotated API credentials
and a changed request model. The OpenAI caller-supplied closure and private registry
refresh closure binding hazard above still requires integration coverage of
refresh=2 grants/sends with one stable bound capability; sibling attempt tests may
supply that coverage, but this target does not.

The fixture authority's seed explicitly acquires twice and completes once; those
are setup transitions, not raw provider attempts. BoundAdmission forwards only to
real public P1 ports. Its manual clock/poll adapter has no production scheduler,
wakeup, queued-drop cleanup or persistence claim. At the final deadline the test
polls/awaits again explicitly; it does not depend on wall time for admission. The
forwarding CurrentRuntimeIngress intentionally ignores proposed admission inputs
until parent replaces it with the alternate production ingress. A test-local
implementation of restart validation/leaf wiring would be false GREEN and must
not be substituted.

## Step-6 oracle counterexamples (subsequent bounded refinement)

Extracted pure observation assertions **used by the actual integration tests**:
`assert_wire_trace`, `assert_budget`, `assert_retained_publication`,
`assert_blocked`, `assert_content`. Added one counterexample test with valid
controls, then 20 individually labelled result/trace perturbations invoking these
same helpers and requiring each to panic via catch_unwind. No production behavior
or enforcement was added to the fixture.

Detected perturbations: unexpected send while expected count zero; missing
independent-group progress; duplicate raw request; wrong wire model; rejected
candidate credential on wire; lost/duplicate active occupancy; absent shared
queue; wrong distinct-group queue; cleared/shortened/reanchored cooldown;
invisible restart requirement; missing/wrong retained error snapshot; candidate
runtime/catalogue generation published; early dispatch; missing/wrong response.

Commands/results:
- `cargo test -p quecto-agentic-harness --test inference_admission_runtime runtime_oracles`
  PASS, 1 test. `/tmp/p2-runtime-oracle-counterexamples.log`.
- Complete runtime target: **16 executed: 2 PASS, 14 expected RED, zero ignored**.
  `/tmp/p2-runtime-with-oracles-red.log`.

This supersedes the earlier assertion-sensitivity limitation only: later
integration assertions remain masked by legitimate initial RED failures, but
these shared oracles now have positive-control and defect-detection evidence.
Counterexamples are not actual production algorithm mutations, not proof of
admission wiring, and do not remove the post-GREEN algorithm mutation obligation.
The pending-future oracle likewise detects a supplied early-ready observation;
only a correctly wired admission barrier can make the real integration observation
meaningful. Existing exact-time public HTTP assertions remain in the integration
path and cannot be replaced by these fabricated observation controls.
