//! `quecto admission-broker run|status|reset`: the same-user shared inference
//! admission authority (#1679 P3). `run` owns the singleton lock and sockets
//! until SIGTERM/SIGINT; `status` and `reset` are owner-only administration.
use super::CliContext;
use crate::infrastructure::admission::{
    AdminConnection, AuthorityDirectory, AuthorityServer, ServerError,
};
use crate::infrastructure::config::Config;

pub(crate) fn cmd_admission_broker(
    ctx: &CliContext,
    args: &[String],
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    crate::infrastructure::logging::install_redacting_subscriber();
    let Some(action) = args.first().map(String::as_str) else {
        stderr.push_str("admission-broker: expected one of run, status, reset\n");
        return 2;
    };
    let base_dir = ctx.base_dir();
    let config = match Config::load(ctx.config_path().to_str().unwrap_or("")) {
        Ok(config) => config.with_admission_base_dir(&base_dir),
        Err(error) => {
            stderr.push_str(&format!("admission-broker: {error}\n"));
            return 1;
        }
    };
    let (directory, proposal) = match config.admission_proposal() {
        Ok(Some(configured)) => configured,
        Ok(None) => {
            stderr.push_str(
                "admission-broker: no `admission` section is configured; nothing to serve\n",
            );
            return 1;
        }
        Err(error) => {
            stderr.push_str(&format!("admission-broker: {error}\n"));
            return 1;
        }
    };
    let accept_missing_ledger = args[1..]
        .iter()
        .any(|flag| flag == "--accept-missing-ledger");
    if let Some(unknown) = args[1..]
        .iter()
        .find(|flag| flag.as_str() != "--accept-missing-ledger")
    {
        stderr.push_str(&format!("admission-broker: unknown option '{unknown}'\n"));
        return 2;
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            stderr.push_str(&format!("admission-broker: runtime: {error}\n"));
            return 1;
        }
    };
    let dir = match AuthorityDirectory::open(&directory) {
        Ok(dir) => dir,
        Err(error) => {
            stderr.push_str(&format!(
                "admission-broker: directory {}: {error}\n",
                directory.display()
            ));
            return 1;
        }
    };
    match action {
        "run" => runtime.block_on(run(dir, proposal, accept_missing_ledger, stdout, stderr)),
        "status" => runtime.block_on(async {
            match AdminConnection::connect(&dir.admin_socket()).await {
                Ok(admin) => match admin.inspect().await {
                    Ok(status) => {
                        stdout.push_str(&status_json(&status).to_string());
                        stdout.push('\n');
                        0
                    }
                    Err(error) => {
                        stderr.push_str(&format!("admission-broker: {error}\n"));
                        1
                    }
                },
                Err(error) => {
                    stderr.push_str(&format!("admission-broker: not running ({error})\n"));
                    1
                }
            }
        }),
        "reset" => runtime.block_on(async {
            match AdminConnection::connect(&dir.admin_socket()).await {
                Ok(admin) => match admin.reset().await {
                    Ok(epoch) => {
                        stdout.push_str(&format!("{{\"epoch\":{epoch}}}\n"));
                        stderr.push_str(
                            "admission-broker: reset acknowledged: old-epoch remote work is no longer claimed bounded; every session must reconnect\n",
                        );
                        0
                    }
                    Err(error) => {
                        stderr.push_str(&format!("admission-broker: {error}\n"));
                        1
                    }
                },
                Err(error) => {
                    stderr.push_str(&format!("admission-broker: not running ({error})\n"));
                    1
                }
            }
        }),
        other => {
            stderr.push_str(&format!(
                "admission-broker: unknown action '{other}' (expected run, status, reset)\n"
            ));
            2
        }
    }
}

fn status_json(status: &crate::application::ports::AuthorityStatus) -> serde_json::Value {
    let groups: serde_json::Map<String, serde_json::Value> = status
        .groups
        .iter()
        .map(|(id, s)| {
            (
                id.as_str().to_owned(),
                serde_json::json!({
                    "active": s.active,
                    "queued": s.queued,
                    "uncertain": s.uncertain,
                    "cooldown_until_ms": s.cooldown_until,
                    "unavailable": s.unavailable,
                }),
            )
        })
        .collect();
    serde_json::json!({
        "epoch": status.epoch,
        "journal_healthy": status.journal_healthy,
        "live_scopes": status.live_scopes,
        "groups": groups,
    })
}

async fn run(
    dir: AuthorityDirectory,
    proposal: crate::infrastructure::provider_runtime_admission::AdmissionRuntimeProposal,
    accept_missing_ledger: bool,
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    let started = if accept_missing_ledger {
        AuthorityServer::start_accepting_missing_ledger(dir, proposal).await
    } else {
        AuthorityServer::start(dir, proposal).await
    };
    let server = match started {
        Ok(server) => server,
        Err(ServerError::Busy(message)) => {
            stderr.push_str(&format!("admission-broker: {message}\n"));
            return 3;
        }
        Err(error) => {
            stderr.push_str(&format!("admission-broker: {error}\n"));
            return 1;
        }
    };
    // Announce readiness on stderr immediately (stdout is buffered by the CLI
    // shell until exit): launch scripts wait for this line or the socket.
    eprintln!(
        "admission authority ready: {}",
        server.directory().client_socket().display()
    );
    let mut sigterm = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    {
        Ok(signal) => signal,
        Err(error) => {
            stderr.push_str(&format!("admission-broker: signal handler: {error}\n"));
            server.shutdown().await;
            return 1;
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = sigterm.recv() => {}
    }
    server.shutdown().await;
    stdout.push_str("admission authority stopped; outstanding work stays journaled\n");
    0
}
