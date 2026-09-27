//! #1942 second review: a configured proxy's name is passed unfiltered by
//! the resolver only so reqwest can reach the proxy itself. A fetch (or a
//! redirect hop) that names the proxy host goes direct, so it is refused:
//! otherwise the exemption would reach whatever the proxy name resolves to.
//! Local listeners only; every client carries an explicit proxy
//! configuration, never the process environment's.
use super::*;
use crate::application::agent_turn::use_cases::web_fetch::ParsedHttpUrl;
use crate::infrastructure::http::proxy_env::proxy_environment;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

fn request(url: &str) -> FetchRequest {
    FetchRequest {
        url: ParsedHttpUrl::parse(url).unwrap(),
    }
}

/// A listener on `ip` answering every connection with `respond(port)`,
/// counting the connections it accepted.
async fn peer_on(
    ip: &str,
    respond: fn(u16) -> String,
) -> (u16, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind((ip, 0)).await.unwrap();
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
                let _ = socket.write_all(respond(port).as_bytes()).await;
            }
        }
    });
    (port, accepted, task)
}

fn ok(_: u16) -> String {
    "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 2\r\n\r\nok".into()
}

fn refused(result: Result<FetchOutcome, FetchFailure>) -> String {
    match result {
        Err(FetchFailure::Refused(reason)) => reason,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// The environment `pairs` describe, as the adapter would read it.
fn environment(pairs: &'static [(&'static str, &'static str)]) -> ProxyEnvironment {
    proxy_environment(false, |name| {
        pairs
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.to_string())
    })
}

/// Every name is admitted, so only the proxy-host rule can refuse
/// `localhost` (the courtesy local-name check would refuse it first).
fn any_name(_: &str) -> Result<(), String> {
    Ok(())
}

fn adapter(
    builder: reqwest::ClientBuilder,
    address: AddressPolicy,
    proxies: ProxyEnvironment,
) -> ReqwestFetchWebContent {
    let policy = DestinationPolicy {
        address,
        name: any_name,
    };
    ReqwestFetchWebContent::with_environment(builder, policy, proxies)
}

/// Finding A: with only an HTTPS proxy named `localhost`, a plain-HTTP
/// fetch of `localhost` goes direct and would be resolved unfiltered.
#[tokio::test]
async fn a_direct_fetch_of_the_https_proxy_host_is_refused_unsent() {
    let (port, accepted, peer) = peer_on("127.0.0.1", ok).await;
    let builder = reqwest::Client::builder()
        .no_proxy()
        .proxy(reqwest::Proxy::https("http://localhost:1").unwrap());
    let result = adapter(
        builder,
        authorize_destination,
        environment(&[("HTTPS_PROXY", "http://localhost:1")]),
    )
    .fetch(&request(&format!("http://localhost:{port}/")))
    .await;
    assert_eq!(
        refused(result),
        "localhost is a configured proxy host, not a fetch target"
    );
    assert_eq!(accepted.load(Ordering::SeqCst), 0, "nothing was sent");
    peer.abort();
}

/// Finding A, reversed: an HTTP proxy and an `https` URL.
#[tokio::test]
async fn a_direct_https_fetch_of_the_http_proxy_host_is_refused_unsent() {
    let (port, accepted, peer) = peer_on("127.0.0.1", ok).await;
    let builder = reqwest::Client::builder()
        .no_proxy()
        .proxy(reqwest::Proxy::http("http://localhost:1").unwrap());
    let result = adapter(
        builder,
        authorize_destination,
        environment(&[("HTTP_PROXY", "http://localhost:1")]),
    )
    .fetch(&request(&format!("https://localhost:{port}/")))
    .await;
    assert_eq!(
        refused(result),
        "localhost is a configured proxy host, not a fetch target"
    );
    assert_eq!(accepted.load(Ordering::SeqCst), 0, "nothing was sent");
    peer.abort();
}

/// Finding B: `NO_PROXY` naming the proxy host sends its fetch direct.
#[tokio::test]
async fn a_no_proxy_fetch_of_the_proxy_host_is_refused_unsent() {
    let (port, accepted, peer) = peer_on("127.0.0.1", ok).await;
    let builder = reqwest::Client::builder().no_proxy().proxy(
        reqwest::Proxy::all("http://localhost:1")
            .unwrap()
            .no_proxy(reqwest::NoProxy::from_string("localhost")),
    );
    let result = adapter(
        builder,
        authorize_destination,
        environment(&[
            ("ALL_PROXY", "http://localhost:1"),
            ("NO_PROXY", "localhost"),
        ]),
    )
    .fetch(&request(&format!("http://localhost:{port}/")))
    .await;
    assert_eq!(
        refused(result),
        "localhost is a configured proxy host, not a fetch target"
    );
    assert_eq!(accepted.load(Ordering::SeqCst), 0, "nothing was sent");
    peer.abort();
}

/// Only 127.0.0.2 beyond public space: the first hop's listener.
fn second_loopback_only(address: IpAddr) -> Result<(), NonPublicAddress> {
    match address {
        IpAddr::V4(v4) if v4 == std::net::Ipv4Addr::new(127, 0, 0, 2) => Ok(()),
        _ => authorize_destination(address),
    }
}

static HOP_PORT: AtomicUsize = AtomicUsize::new(0);

fn redirect_to_proxy_host(_: u16) -> String {
    let port = HOP_PORT.load(Ordering::SeqCst);
    format!(
        "HTTP/1.1 302 Found\r\nConnection: close\r\nLocation: http://localhost:{port}/final\r\nContent-Length: 0\r\n\r\n"
    )
}

/// Finding C: a redirect hop naming the proxy host (an HTTPS-only proxy,
/// so the plain-HTTP hop goes direct).
#[tokio::test]
async fn a_redirect_to_the_proxy_host_is_refused_before_it_is_followed() {
    let (hop_port, hop_accepted, hop) = peer_on("127.0.0.1", ok).await;
    HOP_PORT.store(usize::from(hop_port), Ordering::SeqCst);
    let (port, accepted, peer) = peer_on("127.0.0.2", redirect_to_proxy_host).await;
    let builder = reqwest::Client::builder()
        .no_proxy()
        .proxy(reqwest::Proxy::https("http://localhost:1").unwrap());
    let result = adapter(
        builder,
        second_loopback_only,
        environment(&[("HTTPS_PROXY", "http://localhost:1")]),
    )
    .fetch(&request(&format!("http://127.0.0.2:{port}/start")))
    .await;
    assert_eq!(
        refused(result),
        format!(
            "the redirect to http://localhost:{hop_port}/final: localhost is a configured proxy host, not a fetch target"
        )
    );
    assert_eq!(accepted.load(Ordering::SeqCst), 1, "the first hop only");
    assert_eq!(hop_accepted.load(Ordering::SeqCst), 0, "the hop never sent");
    peer.abort();
    hop.abort();
}

#[test]
fn an_admitted_fetch_host_is_any_host_but_a_proxy_host() {
    let proxies = ["proxy.corp".to_owned()];
    let fetchable = |url: &str| is_admitted_fetch_host(&url::Url::parse(url).unwrap(), &proxies);
    assert!(fetchable("http://example.com/"));
    assert!(fetchable("http://sub.proxy.corp/"));
    assert!(fetchable("http://10.0.0.1/"), "literals are the policy's");
    for url in [
        "http://proxy.corp/",
        "https://PROXY.corp:8443/",
        "http://proxy.corp./",
        "http://proxy.corp../",
    ] {
        assert!(!fetchable(url), "{url}");
    }
}
