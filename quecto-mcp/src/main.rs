use quecto_mcp::{Config, run_extension};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let config = match Config::from_env_and_args(std::env::args()) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("quecto-mcp config error: {err}");
            std::process::exit(2);
        }
    };

    if let Err(err) = run_extension(config).await {
        eprintln!("quecto-mcp error: {}", quecto_mcp::redact(&err.to_string()));
        std::process::exit(1);
    }
}
