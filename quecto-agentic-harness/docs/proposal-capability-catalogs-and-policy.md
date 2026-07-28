# Proposal: Harness-Owned Capability Catalogs and Policy

**Status:** Proposed  
**Related issues:** #1272, #1273, #1274, #1275  
**Scope:** `quecto-agentic-harness`, its UDS interface, and the TUI presentation adapter

## Summary

Quecto should treat models, tools, and workflows as three kinds of agent
capability with a common lifecycle:

```text
Definition sources
    -> typed catalog loaders
    -> validated immutable catalogs
    -> session capability policy
    -> effective catalog views
    -> runtime activation and execution
```

The catalogs answer **what exists**. The policy answers **what an agent in a
session may see and use**. Runtime implementations answer **how an allowed
capability operates**.

The harness owns catalog resolution, policy, persistence, validation, and
enforcement. The TUI is a presentation adapter over capability query and policy
update use cases; it must not own capability truth or write configuration files
directly.

## Motivation

Issues #1272-#1275 propose a reusable TUI modal and model, tool, and workflow
access management. A TUI-first implementation would risk making the selected
sets advisory UI state passed with prompts. That would not protect direct UDS
commands, stale tool calls, spawned agents, or other clients.

The existing capability surfaces also have different shapes:

- models are principally records loaded from `models.json`;
- tools are executable objects registered into a mutable runtime registry;
- workflows are definitions resolved from built-ins, inline configuration,
  explicit or discovered directories, and by-value child assignments.

A shared policy needs stable identities and consistent authorization while
preserving the specialized definitions and execution semantics of each kind.

## Goals

- Keep all capability truth inside the harness.
- Give models, tools, and workflows one consistent catalog-policy lifecycle.
- Make every tool conditional without destructively deleting its definition.
- Resolve workflow definitions into one authoritative catalog per session.
- Allow the TUI to tick and untick session-visible capabilities.
- Persist session overrides and restore them on resume.
- Ensure children can narrow but never broaden their parent's authority.
- Enforce policy both when capabilities are advertised and when they are used.
- Preserve zero-config compatibility: absent policy means all available
  capabilities are permitted, subject to immutable deployment restrictions.

## Non-goals

- Replacing model, tool, and workflow execution with one generic executor.
- Making the TUI a configuration or persistence client.
- Treating a visibility filter as a security sandbox.
- Requiring all capability-specific metadata to fit one generic DTO.
- Silently merging workflow sources with ambiguous precedence.

## Current state

### Configuration

The harness loads `config.json` from an explicit `--config` path or, by default,
`<base-dir>/config.json` (normally `~/.quecto/config.json`). A missing default
file produces typed defaults. Selected environment variables then override
specific fields.

The current root contains agent defaults, providers, web-tool configuration,
and workflow configuration. Most configuration is applied while constructing
the process. Provider/model configuration has partial reload support, but tools,
workflow libraries, and general agent defaults are not dynamically rebuilt.

### Models

`models.json` is loaded into `ModelRegistry { models: Vec<ModelRecord> }`.
`ModelRecord` combines identity, display metadata, API routing, authentication,
limits, input kinds, reasoning support, and pricing. `list_models` projects
these records into manually constructed UDS JSON.

A model is identified by provider and model ID. The canonical capability ID
must be the qualified value:

```text
<provider>/<model-id>
```

A bare model ID is not unique across provider or authentication routes.

### Tools

A tool exposes `ToolDefinition { name, description, parameters_schema }` and an
`execute` operation. `ToolRegistryImpl` stores executable tools by name and a
parallel vector of model-visible definitions. Core, conditional harness,
extension, and UDS-client tools enter the registry through related but not fully
uniform paths.

`--disable-tool` currently removes tools and permanently denies re-registration.
That is an appropriate immutable startup ceiling, but not a live session policy:
an unticked tool cannot subsequently be ticked without reconstruction.

The tool capability ID is its registered name.

### Workflows

`WorkflowTemplate` is the principal definition and its `id` is the capability
identity. A template contains label, description, optional usage guidance,
steps, and guards.

Definitions currently come from:

1. an explicitly configured workflow directory;
2. an auto-discovered repository or user directory;
3. inline `config.json` templates;
4. built-in defaults when the resolved list is empty;
5. a by-value `WorkflowSpec` assigned to a child.

`WorkflowConfig` mixes catalog source selection, inline definitions, selector
prompt configuration, and runtime automation settings. `WorkflowEngine` then
owns both the resolved template library and mutable run state. Workflow
snapshots mix run state with available-template summaries.

There are also several meanings of "workflow enabled": the workflow tool is
visible, templates are visible, automation is enabled, guards are enabled, and
a run may be active. Capability policy in this proposal controls individual
**workflow template visibility**, not those other switches.

## Architectural decision

Use four clean-architecture layers:

```text
Domain
    capability identity, policy, authorization and narrowing rules
    typed model/tool/workflow definitions

Application
    catalog queries, policy queries and updates, effective-view resolution
    model selection, tool invocation, workflow selection and child derivation

Infrastructure
    catalog loaders and registries, credential resolution, runtime factories
    config defaults and session-policy persistence

Interfaces
    UDS and CLI adapters; TUI presentation and modal state
```

The domain defines what access means. Application use cases decide effective
access. Infrastructure discovers, activates, and persists it. Interfaces let
users and clients interact with those use cases.

## Domain shape

### Shared capability vocabulary

```rust
pub enum CapabilityKind {
    Model,
    Tool,
    Workflow,
}

pub struct CapabilityId(String);

pub struct CapabilityDescriptor {
    pub id: CapabilityId,
    pub kind: CapabilityKind,
    pub label: String,
    pub description: Option<String>,
    pub source: CapabilitySource,
    pub availability: CapabilityAvailability,
}

pub enum CapabilitySource {
    BuiltIn,
    User,
    Repository,
    Extension(String),
    ExternalClient(String),
}

pub enum CapabilityAvailability {
    Available,
    Unavailable { reason: String },
}
```

This descriptor is a common projection for application queries and
presentation. It does not replace typed definitions.

### Policy

```rust
pub struct CapabilityPolicy {
    pub models: CapabilitySelection,
    pub tools: CapabilitySelection,
    pub workflows: CapabilitySelection,
}

pub enum CapabilitySelection {
    All,
    Only(BTreeSet<CapabilityId>),
}
```

Semantics:

- `All` permits current and subsequently discovered catalog entries.
- `Only(empty)` permits none.
- unknown IDs are retained in persistence but do not become effective until a
  matching catalog entry exists;
- malformed IDs are rejected;
- catalogs and policy use canonical, case-sensitive IDs.

The effective authority is an intersection:

```text
deployment/startup ceiling
    intersect parent effective policy
    intersect configured session defaults
    intersect session override
```

A child may narrow authority but cannot broaden it.

### Typed definitions

Models, tools, and workflows retain their own domain shapes.

```rust
pub struct ModelDefinition {
    pub id: ModelId,
    pub label: String,
    pub provider: ProviderId,
    pub protocol: ModelProtocol,
    pub authentication: AuthProfileId,
    pub capabilities: ModelCapabilities,
    pub limits: ModelLimits,
    pub pricing: Option<ModelPricing>,
}

pub struct ToolDefinition {
    pub id: ToolId,
    pub label: String,
    pub description: String,
    pub parameters: JsonSchema,
    pub source: ToolSource,
}

pub struct WorkflowDefinition {
    pub id: WorkflowId,
    pub label: String,
    pub description: String,
    pub when_to_use: Option<String>,
    pub steps: Vec<WorkflowStepDefinition>,
    pub guards: Vec<WorkflowGuardDefinition>,
}
```

Secrets do not belong in model definitions. Definitions reference an auth
profile; infrastructure resolves credentials when activating a provider.

Workflow definition remains separate from run state:

```rust
pub struct WorkflowRun {
    pub workflow_id: WorkflowId,
    pub completed_steps: BTreeSet<WorkflowStepId>,
    pub active_issue: Option<IssueReference>,
    pub mode: WorkflowRunMode,
}
```

## Catalog ports

Application logic depends on role-focused catalog ports:

```rust
pub trait ModelCatalog {
    fn all(&self) -> &[ModelDefinition];
    fn get(&self, id: &ModelId) -> Option<&ModelDefinition>;
}

pub trait ToolCatalog {
    fn all(&self) -> &[ToolDefinition];
    fn get(&self, id: &ToolId) -> Option<&ToolDefinition>;
}

pub trait WorkflowCatalog {
    fn all(&self) -> &[WorkflowDefinition];
    fn get(&self, id: &WorkflowId) -> Option<&WorkflowDefinition>;
}
```

A unified `CapabilityCatalog` query projects these typed catalogs into
`CapabilityDescriptor` values. It is not a monolithic replacement for them.

## Models target shape

```text
models.json and built-ins
    -> ModelCatalogLoader
    -> validated ModelCatalog
    -> capability-policy view
    -> ProviderFactory/provider router
```

Responsibilities are separated as follows:

- `models.json`: model and provider definitions;
- credential store, environment, or secret config: authentication material;
- `config.json`: default capability policy and default active model;
- session metadata: mutable session policy override;
- provider runtime: inference execution.

Model policy governs:

- capability and model listings;
- initial and active model selection;
- `set_model`;
- automatic fallback;
- spawned-agent model selection;
- any model discovery exposed to an agent.

Applying a policy that excludes the active model must not leave an illegal
runtime state. The update use case should either atomically select an allowed
replacement supplied by the client or reject the change with a typed conflict.

## Tools target shape

### Every tool is conditional

All tools enter one catalog mechanism, including core, web, spawn, recall,
workflow, extension, and external-client tools. Being built-in does not imply
being unconditionally visible.

Separate registration from session activation:

```text
Built-in factories
Extension factories
External-client registrations
    -> complete ToolCatalog
    -> effective policy view
    -> session ToolExecutor set
```

Some tools need runtime dependencies, so register factories:

```rust
pub trait ToolFactory: Send + Sync {
    fn definition(&self) -> &ToolDefinition;

    fn create(
        &self,
        context: &ToolRuntimeContext,
    ) -> Result<Arc<dyn ToolExecutor>, ToolActivationError>;
}
```

Examples include workspace and sandbox dependencies for file tools, credentials
for web tools, a subagent registry for `spawn`, and workflow state for
`workflow`.

### No destructive session filtering

The complete catalog remains intact. A session registry exposes only effective
definitions and checks policy again before execution:

```rust
pub struct EffectiveToolRegistry {
    catalog: Arc<dyn ToolCatalog>,
    policy: Arc<EffectiveCapabilityPolicy>,
    executors: HashMap<ToolId, Arc<dyn ToolExecutor>>,
}
```

`--disable-tool` remains an immutable startup ceiling represented in policy
composition. It must not be implemented as the mutable TUI selection. This
allows tools to be ticked, unticked, and re-ticked without rebuilding the whole
agent process.

The execution check protects against stale model calls and direct transport
attempts after a tool has been hidden.

## Workflows target shape

### One resolved catalog

All consumers use one validated `WorkflowCatalog`:

```text
Workflow definition source
    -> WorkflowCatalogLoader
    -> validated immutable WorkflowCatalog
       -> WorkflowEngine
       -> capability query
       -> spawn validation
       -> workflow tool
       -> UDS state
```

The engine does not privately invent or supplement its template library. The
loader, not the engine, applies built-in fallback.

### Definition source

Prefer directory-based workflow definitions and keep complete templates out of
operational `config.json`.

```text
~/.quecto/workflows/*.json
<repository>/.quecto/workflows/*.json
```

The simplest deterministic policy selects exactly one catalog root:

```text
explicit configured directory
    -> repository directory
    -> user directory
    -> embedded built-ins
```

Once selected, that source is authoritative. It does not silently merge with or
shadow inline definitions.

If product requirements later need composition, define explicit ordered
sources and reject duplicate IDs by default:

```json
{
  "catalogs": {
    "workflows": {
      "sources": [
        { "kind": "built_in" },
        { "kind": "directory", "path": "~/.quecto/workflows" },
        { "kind": "directory", "path": ".quecto/workflows" }
      ],
      "duplicateIds": "error"
    }
  }
}
```

### By-value child workflows

By-value `WorkflowSpec` must not become an invisible policy bypass. Two valid
end states are:

1. require a catalog `WorkflowId`; or
2. validate the supplied definition into a child-local immutable catalog using
   a content-derived ID such as `ephemeral:sha256:<hash>`.

The second preserves flexible delegation. Its authorization semantics must be
explicit: either the parent policy permits ephemeral workflow grants, or the
parent assignment itself is a narrowly scoped authority grant. Children still
cannot replace or broaden a binding assignment.

### Separate concepts

Keep these independent:

- workflow tool visibility: tool policy;
- workflow template visibility: workflow policy;
- automation and completion nudges: runtime settings;
- guard enforcement: runtime setting and workflow definition;
- active progress: `WorkflowRun` session state.

## Configuration and persistence

A target configuration shape is:

```json
{
  "agents": {
    "defaults": {
      "model": "openai-api/gpt-5.5"
    }
  },
  "catalogs": {
    "models": {
      "path": "~/.quecto/models.json"
    },
    "workflows": {
      "path": ".quecto/workflows"
    },
    "extensions": {
      "paths": ["~/.quecto/extensions"]
    }
  },
  "capabilities": {
    "defaults": {
      "models": {
        "mode": "all"
      },
      "tools": {
        "mode": "only",
        "enabled": [
          "read", "write", "edit", "grep", "find", "spawn", "workflow"
        ]
      },
      "workflows": {
        "mode": "only",
        "enabled": ["feature", "review-pr"]
      }
    }
  },
  "workflow": {
    "auto_continue": true,
    "completion_nudge": true
  }
}
```

This visibly separates catalog locations, policy defaults, and runtime
behaviour.

Persist three distinct things:

```text
Catalog definitions
    durable files describing what exists

Configured default policy
    config.json defaults for new sessions

Session policy override
    session metadata edited through application use cases
```

The TUI does not rewrite `config.json`. Applying a modal change calls the
harness, which validates and atomically persists the session override. Resetting
removes the override and returns the session to configured defaults.

Unknown but well-formed IDs should remain persisted, allowing temporarily
missing extensions or providers to reappear without losing user intent.

## Application services and ports

Suggested use cases:

```text
GetCapabilities
GetCapabilityPolicy
UpdateCapabilityPolicy
ResetCapabilityPolicy
SelectModel
InvokeTool
SelectWorkflow
SpawnAgent
```

Suggested persistence port:

```rust
pub trait CapabilityPolicyRepository {
    fn defaults(&self) -> Result<CapabilityPolicy, PolicyError>;

    fn load_session(
        &self,
        session: &SessionKey,
    ) -> Result<Option<VersionedCapabilityPolicy>, PolicyError>;

    fn save_session(
        &self,
        session: &SessionKey,
        expected_revision: PolicyRevision,
        policy: &CapabilityPolicy,
    ) -> Result<VersionedCapabilityPolicy, PolicyError>;

    fn reset_session(
        &self,
        session: &SessionKey,
        expected_revision: PolicyRevision,
    ) -> Result<VersionedCapabilityPolicy, PolicyError>;
}
```

A shared authorizer prevents separate subsystems from implementing subtly
different rules:

```rust
pub trait CapabilityAuthorizer {
    fn authorize(
        &self,
        subject: &AgentIdentity,
        capability: &CapabilityRef,
    ) -> Result<(), CapabilityDenied>;
}
```

Authorization is required at both boundaries:

1. filter definitions and catalogs before presenting them to an agent;
2. reject selection or execution of a disallowed capability.

Filtering alone is insufficient because direct clients and stale generated
calls may bypass presentation.

## UDS interface

The TUI talks to application use cases through typed UDS adapters, not to the
repository.

Suggested commands:

```json
{"type":"get_capabilities","id":"request-1"}
```

```json
{
  "type":"set_capability_policy",
  "id":"request-2",
  "expectedRevision":7,
  "policy": {
    "models":{"mode":"only","enabled":["openai-api/gpt-5.5"]},
    "tools":{"mode":"only","enabled":["read","grep","find"]},
    "workflows":{"mode":"only","enabled":["feature"]}
  }
}
```

The query result contains:

- versioned configured and effective policy;
- model, tool, and workflow descriptors;
- availability and unavailable reason;
- active model and active workflow conflict information where relevant.

Successful updates emit a `capability_policy_changed` event. Revision checks
prevent two clients from silently overwriting one another.

Do not attach the complete policy to every prompt or follow-up command. It is
session state and should be applied atomically before turns.

## TUI responsibilities

The TUI owns presentation state only:

- request catalog and effective policy;
- display checked, unchecked, and unavailable items;
- fuzzy-filter labels and useful metadata;
- maintain an uncommitted modal working set;
- toggle individual or visible/all entries;
- Apply, Reset, or Cancel;
- display validation, active-capability, and revision conflicts;
- refresh on policy-change events.

A generic modal should compose existing TUI list-navigation, fuzzy-search, and
overlay primitives. It must not own persistence or capability semantics.

Recommended interactions:

- `Space`: toggle highlighted item;
- explicit actions for enable/disable visible items;
- separately labelled actions if enable/disable all catalog items is offered;
- Apply commits the complete working policy;
- Escape/Cancel discards it;
- selection remains stable across filtering;
- dirty and empty states are visible.

## Child-agent inheritance

The parent's effective policy is the child's immutable upper bound:

```text
child effective policy
    = parent effective policy
      intersect child-requested restrictions
```

Consequences:

- explicit child model selection must be parent-visible;
- child `disable_tools` may narrow but not broaden tool access;
- child workflow selection/specification must obey the workflow grant rules;
- a child cannot regain a capability by choosing another config file;
- existing children retain their launch snapshot unless live propagation is
  separately designed and documented.

The `spawn` tool itself is governed by tool policy. Hiding it prevents new
children but does not terminate existing children.

## Failure and consistency rules

- Catalog loading validates duplicate and malformed IDs.
- Explicitly configured missing catalog sources are startup errors.
- Reload failures keep the last-good catalog and surface a warning.
- Policy updates are atomic across models, tools, and workflows.
- An update conflicting with the active model or workflow is rejected unless
  the request includes an explicit valid transition.
- A disabled capability is absent from agent-facing discovery and rejected at
  execution.
- Unavailable catalog entries can be displayed but cannot become effective.
- Secrets never appear in descriptors, UDS payloads, logs, or the TUI.

## Proposed module layout

```text
quecto-agentic-harness/src/
├── domain/
│   ├── capability/
│   │   ├── id.rs
│   │   ├── policy.rs
│   │   └── authorization.rs
│   ├── model/
│   │   ├── definition.rs
│   │   └── id.rs
│   ├── tool/
│   │   ├── definition.rs
│   │   └── execution.rs
│   └── workflow/
│       ├── definition.rs
│       └── run.rs
├── application/
│   ├── capability/
│   │   ├── ports.rs
│   │   ├── query.rs
│   │   └── update_policy.rs
│   ├── model/
│   ├── tool/
│   └── workflow/
├── infrastructure/
│   ├── catalogs/
│   │   ├── model_file_catalog.rs
│   │   ├── tool_factory_catalog.rs
│   │   └── workflow_directory_catalog.rs
│   ├── policy/
│   │   ├── config_defaults.rs
│   │   └── session_repository.rs
│   └── runtime/
│       ├── provider_factory.rs
│       ├── tool_factory_registry.rs
│       └── workflow_engine.rs
└── interface/
    └── cli/
        └── uds_capabilities.rs
```

This is a target organization, not a requirement for a single large move. The
migration should remain incremental and preserve existing public behavior.

## Migration plan

### Phase 1: Domain policy and typed projections

- Introduce capability IDs, selections, policy, and intersection rules.
- Add model/tool/workflow descriptor adapters without changing runtime behavior.
- Add characterization tests for existing catalogs and IDs.

### Phase 2: Query use case and UDS catalog API

- Add application catalog ports and `GetCapabilities`.
- Replace ad hoc model-list JSON with typed transport mapping.
- Add complete tool and workflow catalog projections.

### Phase 3: Non-destructive tool policy

- Introduce tool factories and a complete tool catalog.
- Move all core and conditional tools through the same registration path.
- Replace runtime removal with effective definition filtering and execution
  authorization.
- Retain `--disable-tool` as an immutable ceiling.

### Phase 4: Authoritative workflow catalog

- Move built-in fallback into catalog loading.
- Separate definition source configuration from workflow runtime settings.
- Make the engine, tool, UDS state, and spawn validation consume one catalog.
- Define and migrate by-value child workflow semantics.

### Phase 5: Session policy persistence and enforcement

- Add configured defaults and versioned session overrides.
- Add update/reset use cases and UDS commands.
- Enforce policy in model selection, tool invocation, workflow selection, and
  spawn.

### Phase 6: TUI modal

- Implement the reusable selectable-list modal from #1272.
- Add model, tool, and workflow presentations from #1273-#1275.
- Add revision-conflict and active-capability handling.

### Phase 7: Reload and cleanup

- Make catalog reloads produce validated last-good snapshots.
- Remove superseded inline workflow and destructive visibility paths after a
  compatibility period.
- Update protocol and operator documentation.

## Testing strategy

### Domain tests

- `All` and `Only` semantics;
- empty and unknown-ID behavior;
- policy intersection and child narrowing;
- canonical ID validation;
- active-capability conflict rules.

### Application tests

- query returns catalog plus effective state;
- atomic update, reset, and optimistic-concurrency conflicts;
- child cannot broaden parent policy;
- model, tool, and workflow use cases all consult the same authorizer.

### Infrastructure tests

- catalog source precedence and duplicate detection;
- workflow catalog fallback and reload;
- tool availability based on runtime prerequisites;
- atomic session-policy persistence;
- last-good behavior after malformed reloads.

### Interface and TUI tests

- typed UDS round trips and protocol compatibility;
- no secrets in responses;
- modal filtering, individual toggles, bulk actions, Apply, Reset, and Cancel;
- disabled definitions are not sent to the model;
- stale direct invocations are rejected;
- resume restores session policy.

## Consequences

### Benefits

- One consistent access model across all agent capabilities.
- The harness remains the authority regardless of client.
- All tools can be dynamically and reversibly selected.
- Workflows gain one resolved catalog and clearer definition/run boundaries.
- Subagent inheritance becomes explicit and safe.
- The TUI remains a thin, testable presentation adapter.
- Future capability kinds can reuse the policy vocabulary.

### Costs

- Tool registration must be separated from activation.
- Workflow loading and engine construction require refactoring.
- Session metadata and UDS protocol gain versioned policy state.
- Active model/workflow updates require explicit conflict handling.
- Adapters temporarily coexist with legacy model, tool, and workflow shapes.

## Alternatives considered

### Keep capability selections only in the TUI

Rejected. Other clients and direct UDS commands could bypass them, and session
resume and child inheritance would be inconsistent.

### Send allow-lists with every prompt

Rejected. Capability access is session state, not prompt content. This would
also miss follow-up, selection, execution, and spawn boundaries.

### Destructively remove and re-register tools

Rejected for mutable session policy. It loses definitions, complicates
extension ownership and stateful tools, and makes re-enabling unreliable.

### Put every definition into `config.json`

Rejected. Catalog definitions, secrets, policy, and runtime behavior have
different lifecycles. Workflow and model definitions remain dedicated catalog
inputs; config references catalogs and supplies policy defaults.

### Replace all typed catalogs with one generic capability registry

Rejected. Common discovery and authorization do not erase the specialized
metadata and execution semantics required by models, tools, and workflows.

## Decision rule

> Everything is registered in a typed catalog; everything is conditionally
> exposed by harness-owned policy; everything is authorized again when used;
> and no presentation client owns capability truth.
