use quecto::interface::cli;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    std::process::exit(cli::run(
        args,
        cli::CliComposition {
            web_fetch_tool_factory: quecto::composition::web_fetch::build,
            teardown_graph: quecto::composition::subagent_teardown::build_teardown_graph,
        },
    ));
}
