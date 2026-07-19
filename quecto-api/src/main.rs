use quecto_api::interface::cli::Config;
use quecto_api::interface::server;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config = Config::parse(std::env::args().skip(1), |key| std::env::var(key).ok())
        .unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(1);
        });

    tracing::info!(
        "quecto-api starting on {}:{}, socket: {}",
        config.host,
        config.port,
        config.socket.display()
    );

    let (listener, app) = server::bind(&config).await.unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });

    tracing::info!(
        "quecto-api listening on http://{}",
        listener.local_addr().expect("listener has address")
    );

    if let Err(e) = server::serve(listener, app, shutdown_signal()).await {
        eprintln!("server error: {e}");
        std::process::exit(1);
    }
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .expect("failed to install SIGTERM handler");
    tokio::select! {
        _ = ctrl_c => tracing::info!("received SIGINT"),
        _ = sigterm.recv() => tracing::info!("received SIGTERM"),
    }
}
