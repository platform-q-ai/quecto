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
    let result = ReqwestFetchWebContent::with_policy(DNS_ONLY)
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

/// The resolver's answer for every name: the refused 127.0.0.1 first, so an
/// unfiltered connector would try it first, then the allowed 127.0.0.2.
fn refused_then_allowed(
    _: String,
) -> Pin<Box<dyn Future<Output = std::io::Result<Vec<SocketAddr>>> + Send>> {
    Box::pin(async {
        Ok(vec![
            "127.0.0.1:0".parse().unwrap(),
            "127.0.0.2:0".parse().unwrap(),
        ])
    })
}

/// Only 127.0.0.2 beyond public space.
fn second_loopback_only(address: IpAddr) -> Result<(), NonPublicAddress> {
    match address {
        IpAddr::V4(v4) if v4 == Ipv4Addr::new(127, 0, 0, 2) => Ok(()),
        _ => authorize_destination(address),
    }
}

/// Listeners on 127.0.0.1 and 127.0.0.2 sharing one port (the resolver's
/// answers carry no port; the URL's is used for both).
async fn twin_peers() -> (u16, [Arc<AtomicUsize>; 2], [tokio::task::JoinHandle<()>; 2]) {
    loop {
        let (port, first_accepted, first) = counting_peer(ok).await;
        let Ok(second) = TcpListener::bind(("127.0.0.2", port)).await else {
            first.abort();
            continue;
        };
        let second_accepted = Arc::new(AtomicUsize::new(0));
        let task = tokio::spawn({
            let accepted = second_accepted.clone();
            async move {
                loop {
                    let (mut socket, _) = second.accept().await.unwrap();
                    accepted.fetch_add(1, Ordering::SeqCst);
                    let mut buffer = [0; 4096];
                    let _ = socket.read(&mut buffer).await;
                    let _ = socket.write_all(ok(port).as_bytes()).await;
                }
            }
        });
        return (port, [first_accepted, second_accepted], [first, task]);
    }
}

#[tokio::test]
async fn a_refused_dns_answer_is_never_connected_even_beside_an_allowed_one() {
    let (port, [refused_accepted, allowed_accepted], peers) = twin_peers().await;
    let result = ReqwestFetchWebContent::with_lookup(
        WebFetchClientRecipe::default(),
        DestinationPolicy {
            address: second_loopback_only,
            name: any_name,
        },
        refused_then_allowed,
    )
    .fetch(&request(&format!("http://both.test:{port}/")))
    .await;
    assert!(
        matches!(result, Ok(FetchOutcome::SuccessBody { .. })),
        "{result:?}"
    );
    assert_eq!(
        refused_accepted.load(Ordering::SeqCst),
        0,
        "never connected"
    );
    assert_eq!(
        allowed_accepted.load(Ordering::SeqCst),
        1,
        "the allowed one"
    );
    peers.iter().for_each(|peer| peer.abort());
}

#[test]
fn only_authorized_answers_are_kept_and_an_empty_answer_is_refused() {
    let v4: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let v6: SocketAddr = "[::1]:0".parse().unwrap();
    assert_eq!(
        authorized_answers("mixed.test", vec![v4, v6], ipv6_loopback_only, &[]),
        Ok(vec![v6])
    );
    assert_eq!(
        authorized_answers("none.test", vec![v4], ipv6_loopback_only, &[]),
        Err(Refused(
            "none.test resolves to no public address: 127.0.0.1 is not a public address".into()
        ))
    );
    assert_eq!(
        authorized_answers("empty.test", vec![], ipv6_loopback_only, &[]),
        Err(Refused("empty.test resolves to no address".into()))
    );
}

#[tokio::test]
async fn an_address_literal_is_refused_by_the_adapter_itself_unsent() {
    let (port, accepted, peer) = counting_peer(ok).await;
    let adapter = ReqwestFetchWebContent::with_policy(DestinationPolicy::PRODUCTION);
    for (url, reason) in [
        (
            format!("http://127.0.0.1:{port}/"),
            "127.0.0.1 is not a public address",
        ),
        (
            format!("http://[::ffff:127.0.0.1]:{port}/"),
            "127.0.0.1 (via ::ffff:127.0.0.1) is not a public address",
        ),
        // Ledger row 2: 6to4 is decoded and the embedded address judged.
        (
            format!("http://[2002:7f00:1::1]:{port}/"),
            "127.0.0.1 (via 2002:7f00:1::1) is not a public address",
        ),
        (
            format!("http://[2002:a9fe:a9fe::1]:{port}/"),
            "169.254.169.254 (via 2002:a9fe:a9fe::1) is not a public address",
        ),
        // Ledger rows 1 and 8: space outside the allowed allocations.
        (
            format!("http://240.0.0.1:{port}/"),
            "240.0.0.1 is not a public address",
        ),
        (
            format!("http://[3ffe::1]:{port}/"),
            "3ffe::1 is not a public address",
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
    let result = ReqwestFetchWebContent::with_policy(LOOPBACK_FOR_TESTS)
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
    let result = ReqwestFetchWebContent::with_policy(LOOPBACK_FOR_TESTS)
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
    let hop = |url: &str| {
        authorize_hop(
            &reqwest::Url::parse(url).unwrap(),
            DestinationPolicy::PRODUCTION,
            &[],
        )
    };
    assert_eq!(hop("https://example.com/next"), Ok(()));
    assert_eq!(hop("http://8.8.8.8/"), Ok(()));
    assert_eq!(
        hop("http://10.0.0.1/"),
        Err(FetchFailure::Refused(
            "the redirect to http://10.0.0.1/: 10.0.0.1 is not a public address".into()
        ))
    );
    // A hop is named by its first path segment only (#2248 round 2).
    assert_eq!(
        hop("http://10.0.0.1/cb/SECRET123"),
        Err(FetchFailure::Refused(
            "the redirect to http://10.0.0.1/cb/…: 10.0.0.1 is not a public address".into()
        ))
    );
    // A hop is held to the courtesy local-name check, as the first URL is.
    assert_eq!(
        hop("http://metadata.google.internal./computeMetadata/"),
        Err(FetchFailure::Refused(
            "the redirect to http://metadata.google.internal./computeMetadata/: metadata.google.internal. is a local name".into()
        ))
    );
    assert_eq!(
        hop("ftp://example.com/"),
        Err(FetchFailure::Refused(
            "the redirect to ftp://example.com/: only http and https are fetched".into()
        ))
    );
}

#[tokio::test]
async fn a_client_that_cannot_be_built_fails_every_fetch_closed() {
    let (port, accepted, peer) = counting_peer(ok).await;
    let adapter = ReqwestFetchWebContent {
        client: Err("the web-fetch client could not be built: test".into()),
        policy: LOOPBACK_FOR_TESTS,
        timeout: REQUEST_TIMEOUT,
        proxies_ignored: false,
        nat64: Arc::new(Nat64Discovery {
            lookup: loopback_test_lookup,
            timeout: DISCOVERY_TIMEOUT,
            found: std::sync::OnceLock::new(),
        }),
    };
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
    assert_eq!(admitted_name("example.com"), Ok(()));
    for name in ["localhost", "localhost.", "foo.localhost", "metadata"] {
        assert_eq!(
            admitted_name(name),
            Err(format!("{name} is a local name")),
            "production"
        );
    }
    for name in ["localhost.", "localhost.localdomain", "metadata"] {
        assert!(localhost_or_production(name).is_err(), "{name}");
    }
}
