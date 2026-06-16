use quecto_tui::interface::cli;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    std::process::exit(cli::run(args));
}
