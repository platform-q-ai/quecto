//! #1942: the adapter reaches only authorized addresses — the URL's own
//! address, every DNS answer and every redirect hop — and connects only to
//! the addresses it checked. Local listeners only; no request leaves the
//! machine.
use super::*;
use crate::application::agent_turn::use_cases::web_fetch::ParsedHttpUrl;
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    time::{Duration, timeout},
};

fn request(url: &str) -> FetchRequest {
    FetchRequest {
        url: ParsedHttpUrl::parse(url).unwrap(),
    }
}

/// A peer that answers every connection with `respond(port)` and counts
/// the connections it accepted.
async fn counting_peer(
    respond: fn(u16) -> String,
) -> (u16, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
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

/// Every name admitted: only the resolver's address checks are exercised.
fn any_name(_: &str) -> Result<(), String> {
    Ok(())
}

/// Production addresses, and any name, so a name reaches the resolver.
const DNS_ONLY: DestinationPolicy = DestinationPolicy {
    address: authorize_destination,
    name: any_name,
};

/// Only the IPv6 loopback: every IPv4 answer is refused.
fn ipv6_loopback_only(address: IpAddr) -> Result<(), NonPublicAddress> {
    match address {
        IpAddr::V6(v6) if v6.is_loopback() => Ok(()),
        _ => authorize_destination(address),
    }
}

#[tokio::test]
async fn a_name_resolving_only_to_non_public_addresses_is_refused_unsent() {
    let (port, accepted, peer) = counting_peer(ok).await;
    let result = ReqwestFetchWebContent::without_proxies(DNS_ONLY)
        .fetch(&request(&format!("http://localhost:{port}/secret")))
        .await;
    let reason = refused(result);
    assert!(
        reason.starts_with("localhost resolves to no public address: "),
        "{reason}"
    );
    assert!(
        reason.contains("127.0.0.1 is not a public address"),
        "{reason}"
    );
    assert_eq!(accepted.load(Ordering::SeqCst), 0, "nothing was sent");
    peer.abort();
}

#[tokio::test]
async fn a_refused_dns_answer_is_never_connected_even_beside_an_allowed_one() {
    let (port, accepted, peer) = counting_peer(ok).await;
    // `localhost` answers 127.0.0.1 (refused here) and, where configured,
    // ::1 (allowed, nothing listening): whatever happens to ::1, the
    // refused 127.0.0.1 is never connected.
    let _ = ReqwestFetchWebContent::without_proxies(DestinationPolicy {
        address: ipv6_loopback_only,
        name: any_name,
    })
    .fetch(&request(&format!("http://localhost:{port}/")))
    .await;
    assert_eq!(accepted.load(Ordering::SeqCst), 0);
    peer.abort();
}

#[test]
fn only_authorized_answers_are_kept_and_an_empty_answer_is_refused() {
    let v4: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let v6: SocketAddr = "[::1]:0".parse().unwrap();
    assert_eq!(
        authorized_answers("mixed.test", vec![v4, v6], ipv6_loopback_only),
        Ok(vec![v6])
    );
    assert_eq!(
        authorized_answers("none.test", vec![v4], ipv6_loopback_only),
        Err(Refused(
            "none.test resolves to no public address: 127.0.0.1 is not a public address".into()
        ))
    );
    assert_eq!(
        authorized_answers("empty.test", vec![], ipv6_loopback_only),
        Err(Refused("empty.test resolves to no address".into()))
    );
}

#[tokio::test]
async fn an_address_literal_is_refused_by_the_adapter_itself_unsent() {
    let (port, accepted, peer) = counting_peer(ok).await;
    let adapter = ReqwestFetchWebContent::without_proxies(DestinationPolicy::PRODUCTION);
    for (url, reason) in [
        (
            format!("http://127.0.0.1:{port}/"),
            "127.0.0.1 is not a public address",
        ),
        (
            format!("http://[::ffff:127.0.0.1]:{port}/"),
            "127.0.0.1 (via ::ffff:127.0.0.1) is not a public address",
        ),
        // The courtesy local-name check holds in the adapter too.
        (
            format!("http://localhost:{port}/"),
            "localhost is a local name",
        ),
        (
            format!("http://LocalHost.LocalDomain..:{port}/"),
            "localhost.localdomain.. is a local name",
        ),
    ] {
        assert_eq!(refused(adapter.fetch(&request(&url)).await), reason);
    }
    assert_eq!(accepted.load(Ordering::SeqCst), 0);
    peer.abort();
}

fn redirect_to_mapped_loopback(port: u16) -> String {
    format!(
        "HTTP/1.1 302 Found\r\nConnection: close\r\nLocation: http://[::ffff:127.0.0.1]:{port}/final\r\nContent-Length: 0\r\n\r\n"
    )
}

#[tokio::test]
async fn a_redirect_to_a_non_public_address_is_refused_before_it_is_followed() {
    // The first hop is allowed (the test policy admits 127.0.0.1 only); the
    // redirect names the same listener in its IPv4-mapped spelling, so a
    // followed redirect would be a second connection.
    let (port, accepted, peer) = counting_peer(redirect_to_mapped_loopback).await;
    let result = ReqwestFetchWebContent::without_proxies(LOOPBACK_FOR_TESTS)
        .fetch(&request(&format!("http://localhost:{port}/start")))
        .await;
    assert_eq!(
        refused(result),
        format!(
            "the redirect to http://[::ffff:7f00:1]:{port}/final: 127.0.0.1 (via ::ffff:127.0.0.1) is not a public address"
        )
    );
    assert_eq!(accepted.load(Ordering::SeqCst), 1, "only the first hop");
    peer.abort();
}

fn redirect_to_self(port: u16) -> String {
    format!(
        "HTTP/1.1 302 Found\r\nConnection: close\r\nLocation: http://localhost:{port}/again\r\nContent-Length: 0\r\n\r\n"
    )
}

#[tokio::test]
async fn the_redirect_limit_is_unchanged_at_ten_follows() {
    let (port, accepted, peer) = counting_peer(redirect_to_self).await;
    let result = ReqwestFetchWebContent::without_proxies(LOOPBACK_FOR_TESTS)
        .fetch(&request(&format!("http://localhost:{port}/start")))
        .await;
    match result {
        Err(FetchFailure::Transport(message)) => {
            assert!(message.contains("redirect"), "{message}")
        }
        other => panic!("expected too many redirects, got {other:?}"),
    }
    assert_eq!(
        accepted.load(Ordering::SeqCst),
        11,
        "the request and ten follows"
    );
    peer.abort();
}

#[test]
fn a_redirect_is_authorized_by_scheme_and_address_and_a_name_is_left_to_dns() {
    let proxies = ["proxy.corp".to_owned()];
    let hop = |url: &str| {
        authorize_hop(
            &reqwest::Url::parse(url).unwrap(),
            DestinationPolicy::PRODUCTION,
            &proxies,
        )
    };
    assert_eq!(hop("https://example.com/next"), Ok(()));
    assert_eq!(hop("http://8.8.8.8/"), Ok(()));
    assert_eq!(
        hop("http://10.0.0.1/"),
        Err(Refused(
            "the redirect to http://10.0.0.1/: 10.0.0.1 is not a public address".into()
        ))
    );
    // A hop is held to the courtesy local-name check, as the first URL is.
    assert_eq!(
        hop("http://metadata.google.internal./computeMetadata/"),
        Err(Refused(
            "the redirect to http://metadata.google.internal./computeMetadata/: metadata.google.internal. is a local name".into()
        ))
    );
    assert_eq!(
        hop("http://Proxy.Corp./"),
        Err(Refused(
            "the redirect to http://proxy.corp./: proxy.corp is a configured proxy host, not a fetch target".into()
        ))
    );
    assert_eq!(
        hop("ftp://example.com/"),
        Err(Refused(
            "the redirect to ftp://example.com/: only http and https are fetched".into()
        ))
    );
}

/// A proxy is honoured as before: the request goes to it, and the name is
/// not resolved here (the proxy resolves it). Only address literals are
/// still refused before anything is sent.
#[tokio::test]
async fn a_proxy_on_the_injected_builder_is_used_and_resolves_names_itself() {
    let (port, direct, peer) = counting_peer(ok).await;
    let (proxy_port, proxied, proxy) = counting_peer(ok).await;
    let adapter = ReqwestFetchWebContent::with_environment(
        reqwest::Client::builder()
            .no_proxy()
            .proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{proxy_port}")).unwrap()),
        DNS_ONLY,
        ProxyEnvironment::default(),
    );
    // Direct, `localhost` would be refused (it resolves only to loopback):
    // through the proxy it is not resolved here, so it is sent.
    let result = adapter
        .fetch(&request(&format!("http://localhost:{port}/")))
        .await;
    assert!(
        matches!(result, Ok(FetchOutcome::SuccessBody { .. })),
        "{result:?}"
    );
    assert_eq!(proxied.load(Ordering::SeqCst), 1, "sent to the proxy");
    assert_eq!(direct.load(Ordering::SeqCst), 0, "not connected directly");
    // An address literal is refused before the proxy is reached.
    let literal = adapter
        .fetch(&request(&format!("http://[::ffff:127.0.0.1]:{port}/")))
        .await;
    assert_eq!(
        refused(literal),
        "127.0.0.1 (via ::ffff:127.0.0.1) is not a public address"
    );
    assert_eq!(proxied.load(Ordering::SeqCst), 1, "the literal never left");
    peer.abort();
    proxy.abort();
}

/// The environment reqwest would read for `HTTPS_PROXY=http://localhost:<port>`
/// (and the same for plain HTTP): the builder below carries that proxy, as
/// reqwest would take it from these variables.
fn localhost_proxy_environment() -> crate::infrastructure::http::proxy_env::ProxyEnvironment {
    crate::infrastructure::http::proxy_env::proxy_environment(false, |name| {
        matches!(name, "HTTP_PROXY" | "HTTPS_PROXY").then(|| "http://localhost:3128".to_owned())
    })
}

/// #1942 review: reqwest reaches a proxy through the same connector, so the
/// proxy's own name goes through the resolver. A proxy named by a
/// non-public name (`localhost`, `proxy.corp` on 10.x) is passed unfiltered,
/// so it keeps working as before; every other name is still filtered.
#[tokio::test]
async fn a_proxy_given_by_a_non_public_name_is_still_reached() {
    let (port, direct, peer) = counting_peer(ok).await;
    let (proxy_port, proxied, proxy) = counting_peer(ok).await;
    let builder = || {
        reqwest::Client::builder()
            .no_proxy()
            .proxy(reqwest::Proxy::all(format!("http://localhost:{proxy_port}")).unwrap())
    };
    let adapter = ReqwestFetchWebContent::with_environment(
        builder(),
        DestinationPolicy::PRODUCTION,
        localhost_proxy_environment(),
    );
    let result = adapter
        .fetch(&request(&format!("http://fetch.example:{port}/")))
        .await;
    assert!(
        matches!(result, Ok(FetchOutcome::SuccessBody { .. })),
        "{result:?}"
    );
    assert_eq!(proxied.load(Ordering::SeqCst), 1, "sent to the named proxy");
    assert_eq!(direct.load(Ordering::SeqCst), 0);
    // Without the exemption the proxy's own name is refused, which is why
    // the exemption exists.
    let unexempted = ReqwestFetchWebContent::with_environment(
        builder(),
        DestinationPolicy::PRODUCTION,
        ProxyEnvironment::default(),
    )
    .fetch(&request(&format!("http://fetch.example:{port}/")))
    .await;
    assert!(refused(unexempted).starts_with("localhost resolves to no public address"),);
    assert_eq!(
        proxied.load(Ordering::SeqCst),
        1,
        "never reached unexempted"
    );
    peer.abort();
    proxy.abort();
}

#[test]
fn only_a_proxy_name_is_passed_unfiltered() {
    let v4: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let proxies = ["localhost".to_owned()];
    assert_eq!(
        admitted_answers("localhost", vec![v4], authorize_destination, &proxies),
        Ok(vec![v4])
    );
    assert_eq!(
        admitted_answers("LOCALHOST", vec![v4], authorize_destination, &proxies),
        Ok(vec![v4]),
        "names compare case-insensitively"
    );
    assert_eq!(
        admitted_answers("localhost..", vec![v4], authorize_destination, &proxies),
        Ok(vec![v4]),
        "and without trailing dots"
    );
    assert!(admitted_answers("other.test", vec![v4], authorize_destination, &proxies).is_err());
    assert!(admitted_answers("localhost", vec![], authorize_destination, &proxies).is_err());
}

#[tokio::test]
async fn a_client_that_cannot_be_built_fails_every_fetch_closed() {
    let (port, accepted, peer) = counting_peer(ok).await;
    let adapter = ReqwestFetchWebContent::with_environment(
        reqwest::Client::builder()
            .no_proxy()
            .use_preconfigured_tls(()),
        LOOPBACK_FOR_TESTS,
        ProxyEnvironment::default(),
    );
    let result = timeout(
        Duration::from_secs(5),
        adapter.fetch(&request(&format!("http://localhost:{port}/"))),
    )
    .await
    .unwrap();
    assert!(
        matches!(result, Err(FetchFailure::Transport(_))),
        "{result:?}"
    );
    assert_eq!(accepted.load(Ordering::SeqCst), 0);
    peer.abort();
}

#[test]
fn the_test_policy_admits_only_the_ipv4_loopback_beyond_public_space() {
    assert_eq!(loopback_or_public(IpAddr::V4(Ipv4Addr::LOCALHOST)), Ok(()));
    assert_eq!(loopback_or_public("8.8.8.8".parse().unwrap()), Ok(()));
    for refused in ["127.0.0.2", "::1", "::ffff:127.0.0.1", "10.0.0.1"] {
        assert!(
            loopback_or_public(refused.parse().unwrap()).is_err(),
            "{refused}"
        );
    }
}

#[test]
fn the_test_policy_admits_only_the_name_localhost_beyond_production() {
    assert_eq!(localhost_or_production("localhost"), Ok(()));
    assert_eq!(localhost_or_production("example.com"), Ok(()));
    assert_eq!(not_local_name("example.com"), Ok(()));
    for name in ["localhost", "localhost.", "foo.localhost", "metadata"] {
        assert_eq!(
            not_local_name(name),
            Err(format!("{name} is a local name")),
            "production"
        );
    }
    for name in ["localhost.", "localhost.localdomain", "metadata"] {
        assert!(localhost_or_production(name).is_err(), "{name}");
    }
}
