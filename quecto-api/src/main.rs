use std::path::PathBuf;

use quecto_api::infrastructure::http::router::build_router;
use quecto_api::infrastructure::uds::client::UdsGateway;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let mut socket: Option<PathBuf> = None;
    let mut host = "127.0.0.1".to_string();
    let mut port: u16 = 8080;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--socket" => socket = args.next().map(PathBuf::from),
            "--host" => {
                if let Some(h) = args.next() {
                    host = h;
                }
            }
            "--port" => {
                if let Some(p) = args.next() {
                    port = p.parse().expect("invalid port");
                }
            }
            other => {
                eprintln!("unknown argument: {other}");
                std::process::exit(1);
            }
        }
    }

    let socket = socket
        .or_else(|| std::env::var_os("QUECTO_SOCKET").map(PathBuf::from))
        .unwrap_or_else(|| {
            eprintln!("missing --socket / QUECTO_SOCKET");
            std::process::exit(1);
        });

    tracing::info!(
        "quecto-api starting on {host}:{port}, socket: {}",
        socket.display()
    );

    let gateway = UdsGateway::connect(&socket).await.unwrap_or_else(|e| {
        eprintln!("failed to connect to quecto agent: {e}");
        std::process::exit(1);
    });

    let app = build_router(gateway);
    let addr: std::net::SocketAddr = format!("{host}:{port}").parse().expect("invalid address");

    tracing::info!("quecto-api listening on http://{addr}");

    let listener = tokio::net::TcpListener::bind(addr).await.expect("bind failed");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server error");
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
