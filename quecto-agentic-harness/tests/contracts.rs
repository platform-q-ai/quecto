//! Contract test binary: aggregates every `tests/contracts/{port}.rs` so
//! `cargo test --test contracts` runs the full port-contract suite.

#[path = "contracts/admission_client.rs"]
mod admission_client;
#[path = "contracts/admission_dispatcher.rs"]
mod admission_dispatcher;
#[path = "contracts/admission_fallback.rs"]
mod admission_fallback;
#[path = "contracts/admission_feedback.rs"]
mod admission_feedback;
#[path = "contracts/admission_group_fallback.rs"]
mod admission_group_fallback;
#[path = "contracts/admission_journal.rs"]
mod admission_journal;
#[path = "contracts/admission_observation.rs"]
mod admission_observation;
#[path = "contracts/admission_recovery.rs"]
mod admission_recovery;
#[path = "contracts/admission_registry.rs"]
mod admission_registry;
#[path = "contracts/admission_secret_source.rs"]
mod admission_secret_source;
#[path = "contracts/admission_typed_feedback.rs"]
mod admission_typed_feedback;
#[path = "contracts/admission_typed_feedback_cases.rs"]
mod admission_typed_feedback_cases;
#[path = "contracts/agent_loop.rs"]
mod agent_loop;
#[path = "contracts/attempt_admission.rs"]
mod attempt_admission;
#[path = "contracts/attempt_permit.rs"]
mod attempt_permit;
#[path = "contracts/audit_sink.rs"]
mod audit_sink;
#[path = "contracts/authority_admin.rs"]
mod authority_admin;
#[path = "contracts/authority_service_manager.rs"]
mod authority_service_manager;
#[path = "common/catalogue_conformance.rs"]
mod catalogue_conformance;
#[path = "contracts/catalogue_consumers.rs"]
mod catalogue_consumers;
#[path = "contracts/catalogue_inputs_loader.rs"]
mod catalogue_inputs_loader;
#[path = "contracts/catalogue_source.rs"]
mod catalogue_source;
#[path = "contracts/config_document_store.rs"]
mod config_document_store;
#[path = "contracts/config_document_writer.rs"]
mod config_document_writer;
#[path = "contracts/config_validator.rs"]
mod config_validator;
#[path = "contracts/container_asset_store.rs"]
mod container_asset_store;
#[path = "contracts/container_config_default_equivalence.rs"]
mod container_config_default_equivalence;
#[path = "contracts/container_config_lookup.rs"]
mod container_config_lookup;
#[path = "contracts/container_config_persistence.rs"]
mod container_config_persistence;
#[path = "contracts/container_config_roster.rs"]
mod container_config_roster;
#[path = "contracts/container_runtime_inventory.rs"]
mod container_runtime_inventory;
#[path = "contracts/container_runtime_preflight.rs"]
mod container_runtime_preflight;
#[path = "contracts/container_script_integrity.rs"]
mod container_script_integrity;
#[path = "contracts/context_spill_store.rs"]
mod context_spill_store;
#[path = "contracts/credential_status_port.rs"]
mod credential_status_port;
#[path = "contracts/delegated_children_roster.rs"]
mod delegated_children_roster;
#[path = "contracts/document_lock.rs"]
mod document_lock;
#[path = "contracts/durable_prefix_observation.rs"]
mod durable_prefix_observation;
#[path = "contracts/effective_container_configs.rs"]
mod effective_container_configs;
#[path = "contracts/effort_default_persistence.rs"]
mod effort_default_persistence;
#[path = "contracts/effort_runtime.rs"]
mod effort_runtime;
#[path = "contracts/effort_vocabulary_source.rs"]
mod effort_vocabulary_source;
#[path = "contracts/environment_member_shutdown.rs"]
mod environment_member_shutdown;
#[path = "contracts/environment_process.rs"]
mod environment_process;
#[path = "contracts/environment_process_commands.rs"]
mod environment_process_commands;
#[path = "contracts/environment_registry_store.rs"]
mod environment_registry_store;
#[path = "contracts/extension.rs"]
mod extension;
#[path = "contracts/fleet_settlement.rs"]
mod fleet_settlement;
#[path = "contracts/fresh_session_identity_generator.rs"]
mod fresh_session_identity_generator;
#[path = "contracts/historical_roster_source.rs"]
mod historical_roster_source;
#[path = "contracts/hosted_swarm_run_inspection.rs"]
mod hosted_swarm_run_inspection;
#[path = "contracts/hosted_swarm_run_observation.rs"]
mod hosted_swarm_run_observation;
#[path = "contracts/llm_provider.rs"]
mod llm_provider;
#[path = "contracts/loaded_catalogue_inputs.rs"]
mod loaded_catalogue_inputs;
#[path = "contracts/loaded_refresh_inputs.rs"]
mod loaded_refresh_inputs;
#[path = "contracts/overlay_trust_store.rs"]
mod overlay_trust_store;
#[path = "contracts/provider_runtime_factory.rs"]
mod provider_runtime_factory;
#[path = "contracts/refresh_inputs_loader.rs"]
mod refresh_inputs_loader;
#[path = "contracts/refresh_redaction_port.rs"]
mod refresh_redaction_port;
#[path = "contracts/refreshable_catalogue_source.rs"]
mod refreshable_catalogue_source;
#[path = "contracts/reload_runtime.rs"]
mod reload_runtime;
#[path = "contracts/retained_context.rs"]
mod retained_context;
#[path = "contracts/runtime_composition.rs"]
mod runtime_composition;
#[path = "contracts/runtime_configuration_source.rs"]
mod runtime_configuration_source;
#[path = "contracts/runtime_snapshot_source.rs"]
mod runtime_snapshot_source;
#[path = "contracts/runtime_tool_lifecycle_registry.rs"]
mod runtime_tool_lifecycle_registry;
#[path = "contracts/session_aware_tools.rs"]
mod session_aware_tools;
#[path = "contracts/session_export_port.rs"]
mod session_export_port;
#[path = "contracts/session_key_propagation.rs"]
mod session_key_propagation;
#[path = "contracts/session_layout.rs"]
mod session_layout;
#[path = "contracts/session_list_scale.rs"]
mod session_list_scale;
#[path = "contracts/session_store.rs"]
mod session_store;
#[path = "contracts/session_switch_runtime.rs"]
mod session_switch_runtime;
#[path = "contracts/subagent_launch_ports.rs"]
mod subagent_launch_ports;
#[path = "common/switch_runtime_fixture.rs"]
mod switch_runtime_fixture;
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
#[path = "contracts/turn_accounting_reset.rs"]
mod turn_accounting_reset;

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

#[path = "contracts/request_accounting.rs"]
mod request_accounting;
#[path = "contracts/request_admission.rs"]
mod request_admission;
#[path = "common/swarm_control_fixture.rs"]
mod swarm_control_fixture;
#[path = "contracts/swarm_run_control.rs"]
mod swarm_run_control;
#[path = "contracts/tool_execution_admission.rs"]
mod tool_execution_admission;

#[path = "contracts/find_paths.rs"]
mod find_paths;

// Subagent teardown ports (#1934).
#[path = "contracts/composition_exit_readiness.rs"]
mod composition_exit_readiness;
#[path = "contracts/delegated_agent_registry.rs"]
mod delegated_agent_registry;
#[path = "contracts/direct_child_routing.rs"]
mod direct_child_routing;
#[path = "contracts/fetch_web_content.rs"]
mod fetch_web_content;
#[path = "contracts/model_default_persistence.rs"]
mod model_default_persistence;
#[path = "contracts/model_runtime.rs"]
mod model_runtime;
#[path = "contracts/owned_child_termination.rs"]
mod owned_child_termination;
#[path = "contracts/shutdown_clock.rs"]
mod shutdown_clock;
#[path = "contracts/shutdown_run_spawner.rs"]
mod shutdown_run_spawner;
#[path = "contracts/shutdown_session_persistence.rs"]
mod shutdown_session_persistence;
#[path = "contracts/subagent_lifecycle_repository.rs"]
mod subagent_lifecycle_repository;
#[path = "contracts/teardown_compensation.rs"]
mod teardown_compensation;
#[path = "common/teardown_fixture.rs"]
mod teardown_fixture;
#[path = "contracts/teardown_loop_adapters.rs"]
mod teardown_loop_adapters;
#[path = "contracts/turn_cancellation.rs"]
mod turn_cancellation;
#[path = "contracts/uds_direct_child_routing.rs"]
mod uds_direct_child_routing;
#[path = "contracts/workflow_run_source.rs"]
mod workflow_run_source;

#[path = "contracts/resume_decision_listing.rs"]
mod resume_decision_listing;
#[path = "contracts/resume_fixture.rs"]
mod resume_fixture;
#[path = "contracts/session_home_catalogue.rs"]
mod session_home_catalogue;
#[path = "contracts/session_metadata_search.rs"]
mod session_metadata_search;
#[path = "contracts/session_rejection_cache.rs"]
mod session_rejection_cache;
#[path = "contracts/workspace_discovery.rs"]
mod workspace_discovery;
#[path = "contracts/workspace_origin.rs"]
mod workspace_origin;
