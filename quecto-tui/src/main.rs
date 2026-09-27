use quecto_tui::shell::cli;

fn main() {
    quecto_fail_fast::abort_on_panic();
    let args: Vec<String> = std::env::args().collect();
    std::process::exit(cli::run(args));
}
