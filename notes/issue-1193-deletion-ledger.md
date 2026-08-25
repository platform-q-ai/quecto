# Issue #1193 deletion/consolidation ledger

## Deleted production lines

No behaviour-enforcing production lines were deleted in this slice. The refactor adds canonical domain/application seams and reroutes existing CLI-owned entry points through those seams while preserving legacy bodies behind adapters.

## Moved / made private

- `build_agent_provider` remains public as a compatibility façade. Its old body moved into private `compose_agent_provider`.
  - Legacy invariant preserved: built-in OpenAI/Anthropic config, registry provider construction, OAuth wrapping, endpoint limits, duplicate prefix rejection, remote-HTTP rejection, malformed `models.json`, and no-provider errors.
  - New location of invariant: unchanged body of `compose_agent_provider` plus application `ProviderRuntimeFactory`/`ComposeProviderRuntimeUseCase` seam.
  - Frozen anchors: `cargo test -p quecto-agentic-harness --lib agent_provider`.
- CLI `models discover` now calls application `RefreshCatalogueSourceUseCase` through `CliCatalogueRefreshPort`; the old discovery helper remains private as the infrastructure-facing adapter body.
  - Legacy invariant preserved: current command syntax/messages, OpenAI-compatible discovery only, OAuth unsupported, unsafe URL rejection before auth, dedupe-by-id, bounded responses, target-provider replacement, atomic publisher seam.
  - Frozen anchors: `cargo test -p quecto-agentic-harness --lib discover`.

## Consolidation completeness

- Canonical provider/model identity now lives in `domain::catalogue::{ProviderId, ModelId, ModelRef}`.
  - Existing legacy string IDs remain at external interfaces for compatibility and migration; they are not reimplemented as a competing authority.
- Application-owned use cases/ports now exist for catalogue resolve/query/selection/refresh/runtime composition.
  - `interface::cli::models` is routed through `RefreshCatalogueSourceUseCase`.
  - `interface::cli::agent_provider::build_agent_provider` is routed through `ComposeProviderRuntimeUseCase`.
  - `infrastructure::catalogue_registry::ModelRegistryCatalogueSource` adapts legacy `ModelRegistry` to domain descriptors.
- Declined consolidation in this slice: `ModelRegistry` remains in infrastructure and UDS list projection still reads it directly for compatibility. The new adapter creates the migration seam; full removal of direct reads requires broader interface/runtime publication changes and additional tests.

## Additional step 6 consolidation entries

- Moved concrete provider runtime construction out of `interface/cli/agent_provider.rs` into `infrastructure/provider_runtime.rs`.
  - Deleted interface-owned provider-construction body invariant: configured built-in provider selection, registry provider construction, OAuth refresh wrapping, retry wrapping, endpoint count/duplicate/remote-HTTP validation, Google unsupported transport rejection, and malformed `models.json` surfacing.
  - Re-established in: `infrastructure::provider_runtime::{InfrastructureProviderRuntimeFactory, build_agent_provider, compose_agent_provider}` behind application `ProviderRuntimeFactory`.
  - Interface compatibility: `interface::cli::agent_provider::build_agent_provider` is now a thin façade through `ComposeProviderRuntimeUseCase`.
  - Frozen anchors: `cargo test -p quecto-agentic-harness --lib agent_provider` and `cargo test -p quecto-agentic-harness --lib provider_runtime`.
- Moved CLI discovery helper implementation out of `interface/cli/models.rs` into `infrastructure/catalogue_discovery.rs`.
  - Deleted interface-owned discovery invariant: OpenAI-compatible `/v1/models` URL construction, auth-token resolution, response byte/model caps, id dedupe/sort, re-read-before-publish, atomic write mode, and redacted errors.
  - Re-established in: `ModelsJsonCatalogueRefreshAdapter` and helper functions behind application `CatalogueRefreshPort`/`RefreshCatalogueSourceUseCase`.
  - Frozen-test adaptation: `interface/cli/models_tests.rs` imports the moved infrastructure helpers directly; no assertions were removed. Mutation evidence below refreshed this adaptation.
- Routed UDS list-models through `ResolveCatalogueUseCase` + `ModelRegistryCatalogueSource`.
  - Deleted UDS direct registry projection invariant: response shape/order/fields and configured flag.
  - Re-established in: domain descriptors from the infrastructure registry adapter plus UDS projection from descriptors.
  - Frozen anchor: `cargo test -p quecto-agentic-harness --lib uds_models`.
- Routed agent startup, UDS set_model, and REPL model limit lookups through application `ResolveModelLimitsUseCase` and infrastructure `ModelRegistryLimitSource`.
  - Deleted direct interface calls to `ModelRegistry::model_limits_from_base_dir`.
  - Re-established in: `infrastructure::catalogue_limits::model_limits_from_base_dir` using typed `ModelRef` and the application `ModelLimitSource` port.
  - Frozen/new anchors: `model_registry`, `catalogue_limits`, #935/#1048 tests where present.
- Split speculative refresh/runtime structs out of the broad catalogue use-case module.
  - Removed unused `RefreshableCatalogueSource`, `RefreshCatalogueUseCase`, `SourceStatus`, `RefreshOutcome`, `RuntimeComposer`, `RuntimeSnapshot`, and `ComposeRuntimeUseCase` from `application/catalogue.rs`.
  - Re-established refresh in: `application/catalogue_refresh.rs`, consumed by CLI and UDS refresh paths.
  - Re-established generation-consistent runtime helper in: `application/catalogue_runtime.rs` with its own focused tests.

## Final architecture decision

The CLI is the outer composition root. The composition root may instantiate infrastructure adapters while invoking application-owned policy and use cases. Such composition-root infrastructure imports are intentional; dependency direction remains inward because domain and application do not import the interface layer.

## Removed unused seams

The production-unused `ResolveCatalogueUseCase`, `CatalogueSource`, `ComposeCatalogueRuntimeUseCase`, and `CatalogueRuntimeComposer` abstractions and their artificial contract tests were removed. The atomic `CatalogueRuntimeSnapshot` remains application-owned with provider runtime composition and is published through `AgentLoop::swap_runtime`.

## Deferred follow-up work

Separating runtime connection policy (`base_url`, `auth_header`, and `allow_remote_http`) from canonical model descriptors is deferred to a focused follow-up. Decomposing `infrastructure/provider_runtime.rs` into descriptor derivation, credential availability, provider construction, and router assembly is also deferred; neither structural redesign belongs in this merge-cleanup slice.
