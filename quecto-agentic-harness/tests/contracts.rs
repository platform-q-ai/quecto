//! Contract test binary: aggregates every `tests/contracts/{port}.rs` so
//! `cargo test --test contracts` runs the full port-contract suite.

#[path = "contracts/agent_loop.rs"]
mod agent_loop;
#[path = "contracts/audit_sink.rs"]
mod audit_sink;
#[path = "contracts/context_spill_store.rs"]
mod context_spill_store;
#[path = "contracts/extension.rs"]
mod extension;
#[path = "contracts/extension_tool_registry.rs"]
mod extension_tool_registry;
#[path = "contracts/llm_provider.rs"]
mod llm_provider;
#[path = "contracts/session_aware_tools.rs"]
mod session_aware_tools;
#[path = "contracts/session_store.rs"]
mod session_store;
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
