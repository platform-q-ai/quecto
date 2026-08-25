//! Contract test binary: aggregates every `tests/contracts/{port}.rs` so
//! `cargo test --test contracts` runs the full port-contract suite.

#[path = "contracts/agent_loop.rs"]
mod agent_loop;
#[path = "contracts/audit_sink.rs"]
mod audit_sink;
#[path = "contracts/catalogue_refresh_all_port.rs"]
mod catalogue_refresh_all_port;
#[path = "contracts/catalogue_refresh_port.rs"]
mod catalogue_refresh_port;
#[path = "contracts/context_spill_store.rs"]
mod context_spill_store;
#[path = "contracts/environment_finalization_port.rs"]
mod environment_finalization_port;
#[path = "contracts/environment_kill_port.rs"]
mod environment_kill_port;
#[path = "contracts/extension.rs"]
mod extension;
#[path = "contracts/llm_provider.rs"]
mod llm_provider;
#[path = "contracts/model_limit_source.rs"]
mod model_limit_source;
#[path = "contracts/provider_runtime_factory.rs"]
mod provider_runtime_factory;
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
