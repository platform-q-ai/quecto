use quecto::interface::cli;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    std::process::exit(cli::run_with_web_fetch_factory(
        args,
        Some(quecto::composition::web_fetch::build),
    ));
}
