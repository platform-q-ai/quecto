//! Contract test binary: aggregates every `tests/contracts/{port}.rs` so
//! `cargo test --test contracts` runs the full port-contract suite.

#[path = "contracts/admission_client.rs"]
mod admission_client;
#[path = "contracts/admission_dispatcher.rs"]
mod admission_dispatcher;
#[path = "contracts/admission_registry.rs"]
mod admission_registry;
#[path = "contracts/agent_loop.rs"]
mod agent_loop;
#[path = "contracts/audit_sink.rs"]
mod audit_sink;
#[path = "common/catalogue_conformance.rs"]
mod catalogue_conformance;
#[path = "contracts/catalogue_consumers.rs"]
mod catalogue_consumers;
#[path = "contracts/catalogue_source.rs"]
mod catalogue_source;
#[path = "contracts/context_spill_store.rs"]
mod context_spill_store;
#[path = "contracts/credential_status_port.rs"]
mod credential_status_port;
#[path = "contracts/environment_finalization_port.rs"]
mod environment_finalization_port;
#[path = "contracts/environment_kill_port.rs"]
mod environment_kill_port;
#[path = "contracts/extension.rs"]
mod extension;
#[path = "contracts/llm_provider.rs"]
mod llm_provider;
#[path = "contracts/provider_runtime_factory.rs"]
mod provider_runtime_factory;
#[path = "contracts/refresh_redaction_port.rs"]
mod refresh_redaction_port;
#[path = "contracts/refreshable_catalogue_source.rs"]
mod refreshable_catalogue_source;
#[path = "contracts/runtime_composition.rs"]
mod runtime_composition;
#[path = "contracts/runtime_tool_lifecycle_registry.rs"]
mod runtime_tool_lifecycle_registry;
#[path = "contracts/session_aware_tools.rs"]
mod session_aware_tools;
#[path = "contracts/session_store.rs"]
mod session_store;
#[path = "contracts/subagent_launch_ports.rs"]
mod subagent_launch_ports;
#[path = "contracts/tool.rs"]
mod tool;
#[path = "contracts/tool_catalog.rs"]
mod tool_catalog;
#[path = "contracts/tool_executor.rs"]
mod tool_executor;
#[path = "contracts/tool_guard.rs"]
mod tool_guard;
#[path = "contracts/tool_policy_mutator.rs"]
mod tool_policy_mutator;
#[path = "contracts/tool_registry.rs"]
mod tool_registry;

#[path = "contracts/clock.rs"]
mod clock;
#[path = "contracts/coordination_port.rs"]
mod coordination_port;
#[path = "contracts/process_control.rs"]
mod process_control;
#[path = "contracts/process_observation.rs"]
mod process_observation;
#[path = "contracts/swarm_lifecycle.rs"]
mod swarm_lifecycle;
