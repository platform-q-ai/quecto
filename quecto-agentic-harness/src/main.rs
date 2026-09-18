use quecto::interface::cli;

fn main() {
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
            catalogue: quecto::composition::catalogue::build_catalogue_handles,
            provider_runtime: quecto::composition::runtime::build_agent_provider,
            tool_policy_persistence:
                quecto::composition::tool_policy::build_tool_policy_persistence,
            container_config_selection:
                quecto::composition::container_configs::build_agent_container_config_selection,
            container_doctor: quecto::composition::environments::build_container_doctor,
        },
    ));
}
