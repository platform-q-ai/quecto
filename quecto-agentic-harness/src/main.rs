use quecto::interface::cli;

fn main() {
    // Fail-fast outside tool calls; a panicking tool costs only its call
    // (#2192, ADR-0029).
    quecto::interface::panic_hook::install();
    let args: Vec<String> = std::env::args().collect();
    std::process::exit(cli::run(
        args,
        cli::CliComposition {
            web_fetch_tool_factory: quecto::composition::web_fetch::build,
            teardown_graph: quecto::composition::subagent_teardown::build_teardown_graph,
            kill_tool: quecto::composition::subagent_termination::install_termination_owners,
            sessions: quecto::composition::sessions::build_session_handles,
            retention: quecto::composition::sessions::build_retention_handles,
            fresh_session_identity: quecto::composition::sessions::build_fresh_session_identity,
            configuration: quecto::composition::configuration::build_configuration_handles,
            admission: quecto::composition::admission::build_admission_handles,
            catalogue: quecto::composition::catalogue::build_catalogue_handles,
            provider_runtime: quecto::composition::runtime::build_agent_provider,
            tool_policy_persistence:
                quecto::composition::tool_policy::build_tool_policy_persistence,
            container_configs:
                quecto::composition::container_configs::build_agent_container_config_handles,
            container_doctor: quecto::composition::environments::build_container_doctor,
            environment_registry: quecto::composition::environments::build_environment_registry,
            container_inventory: quecto::composition::environments::build_container_inventory,
            container_init: quecto::composition::standard_container::build_standard_container_init,
            container_status: quecto::composition::standard_container::build_container_status,
            run_end_fleet: quecto::composition::subagent_teardown::build_run_end_fleet,
            swarm_board: quecto::composition::swarm::build_swarm_board_handles,
            swarm_board_wire: quecto::composition::swarm::board_wire(),
            swarm_board_log: quecto::composition::swarm::board_op_log,
            claude_member: quecto::composition::claude_member::build_claude_member_handles,
        },
    ));
}
