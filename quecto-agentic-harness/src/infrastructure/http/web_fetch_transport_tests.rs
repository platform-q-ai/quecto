//! #1942 (ledger rows 11-13): web_fetch reaches its destination only by a
//! direct connection to an authorized address. It never uses a proxy,
//! whatever the environment says, and no caller can hand it a transport:
//! its client is built from an owned [`WebFetchClientRecipe`]. The proxy
//! environment is set only in a child test process, never in this one.
//! Local listeners only.
use super::*;
use std::process::Stdio;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

/// A listener answering every connection with a 200 `body`, counting the
/// connections it accepted.
async fn counting_peer(body: &'static str) -> (u16, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let accepted = Arc::new(AtomicUsize::new(0));
    let task = tokio::spawn({
        let accepted = accepted.clone();
        async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                accepted.fetch_add(1, Ordering::SeqCst);
                let mut buffer = [0; 4096];
                let _ = socket.read(&mut buffer).await;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
            }
        }
    });
    (port, accepted, task)
}

/// Set only in the child process: the URL it fetches.
const CHILD_TARGET: &str = "QUECTO_WEB_FETCH_CHILD_TARGET";
const IN_CHILD: &str =
    "infrastructure::http::web_fetch::transport_tests::fetch_under_the_inherited_proxy_environment";

/// In the child (see [`every_proxy_variable_is_ignored_and_the_target_reached_directly`])
/// fetches [`CHILD_TARGET`] through the production composition path under
/// the inherited proxy variables; elsewhere does nothing.
#[tokio::test]
async fn fetch_under_the_inherited_proxy_environment() {
    let Some(target) = std::env::var_os(CHILD_TARGET) else {
        return;
    };
    let target = target.into_string().expect("a UTF-8 URL");
    let tool = crate::composition::web_fetch::build_allowing_loopback_for_tests(
        crate::interface::shared::web_fetch_client_recipe(),
        32,
    );
    let arguments = serde_json::json!({ "url": target, "raw": true }).to_string();
    let result = tool.execute(&arguments).await.expect("the tool runs");
    println!(
        "child result: error={} content={}",
        result.is_error, result.content
    );
}

/// Runs the child fetch of `target` with every proxy variable set to
/// `proxy`, returning its output.
async fn child_fetch(target: &str, proxy: &str) -> String {
    let exe = std::env::current_exe().expect("the test binary");
    let mut command = tokio::process::Command::new(exe);
    command
        .args([IN_CHILD, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD_TARGET, target)
        .env_remove("NO_PROXY")
        .env_remove("no_proxy")
        .stdin(Stdio::null());
    for variable in PROXY_VARIABLES {
        command.env(variable, proxy);
    }
    let output = command.output().await.expect("the child test runs");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "{}\n{stdout}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    stdout
}

/// H1 / ledger row 13: with every proxy variable naming a live local proxy,
/// the fetch never touches it and reaches the checked target directly.
#[tokio::test]
async fn every_proxy_variable_is_ignored_and_the_target_reached_directly() {
    let (proxy_port, proxied, proxy) = counting_peer("from the proxy").await;
    let (port, direct, peer) = counting_peer("direct").await;
    let stdout = child_fetch(
        &format!("http://{TEST_HOST}:{port}/"),
        &format!("http://127.0.0.1:{proxy_port}"),
    )
    .await;
    assert!(
        stdout.contains("child result: error=false content=direct"),
        "{stdout}"
    );
    assert_eq!(proxied.load(Ordering::SeqCst), 0, "the proxy is never used");
    assert_eq!(direct.load(Ordering::SeqCst), 1, "reached directly");
    proxy.abort();
    peer.abort();
}

/// Through a proxy a name would be resolved by the proxy, unchecked: here a
/// name whose every address is refused is refused, proxy variables or not.
#[tokio::test]
async fn a_refused_destination_stays_refused_under_proxy_variables() {
    let (proxy_port, proxied, proxy) = counting_peer("from the proxy").await;
    let (port, direct, peer) = counting_peer("direct").await;
    // 127.0.0.2 is refused by the test policy (only 127.0.0.1 is admitted);
    // as an address literal it needs no DNS.
    let stdout = child_fetch(
        &format!("http://127.0.0.2:{port}/"),
        &format!("http://127.0.0.1:{proxy_port}"),
    )
    .await;
    assert!(
        stdout.contains("child result: error=true")
            && stdout.contains("127.0.0.2 is not a public address"),
        "{stdout}"
    );
    assert_eq!(proxied.load(Ordering::SeqCst), 0);
    assert_eq!(direct.load(Ordering::SeqCst), 0);
    proxy.abort();
    peer.abort();
}

/// Proxy values hyper-util keeps with an empty host once panicked the build
/// (#1942 review 3); they are now simply ignored.
#[tokio::test]
async fn unusable_proxy_values_neither_panic_nor_divert_the_fetch() {
    let (port, direct, peer) = counting_peer("direct").await;
    for proxy in [
        "http://:3128",
        ":3128",
        "http://user@:3128",
        "socks5://:1",
        "http://[]:1",
    ] {
        let stdout = child_fetch(&format!("http://{TEST_HOST}:{port}/"), proxy).await;
        assert!(
            stdout.contains("child result: error=false content=direct"),
            "{proxy}: {stdout}"
        );
    }
    assert_eq!(direct.load(Ordering::SeqCst), 5);
    peer.abort();
}

#[test]
fn the_ignored_proxy_note_names_every_variable_set() {
    assert_eq!(ignored_proxies_note(|_| false), None);
    assert_eq!(
        ignored_proxies_note(|name| matches!(name, "HTTPS_PROXY" | "all_proxy")).as_deref(),
        Some(
            "web_fetch connects directly and ignores HTTP(S)_PROXY/ALL_PROXY so every destination is checked (#1942); set: HTTPS_PROXY, all_proxy"
        )
    );
    assert_eq!(
        PROXY_VARIABLES,
        [
            "HTTP_PROXY",
            "http_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "ALL_PROXY",
            "all_proxy"
        ]
    );
}

/// H2 / ledger rows 11-12: the recipe carries only the settings production
/// chooses; the adapter builds everything else itself.
#[test]
fn the_production_recipe_is_the_shared_connect_timeout_and_built_in_roots() {
    let recipe = crate::interface::shared::web_fetch_client_recipe();
    assert_eq!(
        recipe.connect_timeout,
        Some(crate::infrastructure::providers::CONNECT_TIMEOUT)
    );
    assert_eq!(
        crate::infrastructure::providers::CONNECT_TIMEOUT,
        Duration::from_secs(10)
    );
    assert!(recipe.root_certificates.is_empty());
}

/// Ledger row 3: the whole-fetch deadline is unchanged at ten seconds.
#[test]
fn the_production_deadline_is_ten_seconds() {
    let adapter = ReqwestFetchWebContent::new(WebFetchClientRecipe::default());
    assert_eq!(adapter.timeout, Duration::from_secs(10));
    assert!(adapter.client.is_ok());
}
