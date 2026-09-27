//! #1942 second review: a network-specific NAT64 prefix is discovered
//! (RFC 7050, `ipv4only.arpa` through the adapter's own lookup) and every
//! address under it is judged by the IPv4 address the translator reaches:
//! the URL's literal, every DNS answer and every redirect hop. Discovery
//! that fails or is slow means no prefix, and never holds a fetch past its
//! deadline. Injected answers only: nothing is looked up or sent off the
//! machine.
use super::*;
use crate::application::agent_turn::use_cases::web_fetch::ParsedHttpUrl;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    time::sleep,
};

fn request(url: &str) -> FetchRequest {
    FetchRequest {
        url: ParsedHttpUrl::parse(url).unwrap(),
    }
}

fn any_name(_: &str) -> Result<(), String> {
    Ok(())
}

/// The test policy's addresses (127.0.0.1 beyond public space), any name.
const LOOPBACK: DestinationPolicy = DestinationPolicy {
    address: loopback_or_public,
    name: any_name,
};

type Answer = Pin<Box<dyn Future<Output = std::io::Result<Vec<SocketAddr>>> + Send>>;

/// The reviewer's /96 network-specific prefix (inside allocated space).
fn slash96() -> Nat64Prefix {
    Nat64Prefix::new("2001:4860:1234:5678:9abc:de00::".parse().unwrap(), 96).unwrap()
}

/// A /64 network-specific prefix, whose IPv4 bits skip the u octet.
fn slash64() -> Nat64Prefix {
    Nat64Prefix::new("2001:4860:64:64::".parse().unwrap(), 64).unwrap()
}

fn answer(address: Ipv6Addr) -> SocketAddr {
    SocketAddr::from((address, 0))
}

/// A DNS64 network translating through `prefix`: `ipv4only.arpa` reveals
/// it, `private.test` is 10.0.0.1 behind it, `metadata.test` is
/// 169.254.169.254 behind it, and every other name is 127.0.0.1.
fn dns64(prefix: Nat64Prefix, host: &str) -> Vec<SocketAddr> {
    let behind = |a, b, c, d| vec![answer(prefix.synthesize(Ipv4Addr::new(a, b, c, d)))];
    match host {
        DISCOVERY_NAME => vec![
            answer(prefix.synthesize(Ipv4Addr::new(192, 0, 0, 170))),
            answer(prefix.synthesize(Ipv4Addr::new(192, 0, 0, 171))),
        ],
        "private.test" => behind(10, 0, 0, 1),
        "metadata.test" => behind(169, 254, 169, 254),
        _ => vec![SocketAddr::from(([127, 0, 0, 1], 0))],
    }
}

fn dns64_slash96(host: String) -> Answer {
    let answers = dns64(slash96(), &host);
    Box::pin(async move { Ok(answers) })
}

fn dns64_slash64(host: String) -> Answer {
    let answers = dns64(slash64(), &host);
    Box::pin(async move { Ok(answers) })
}

fn adapter(lookup: Lookup) -> ReqwestFetchWebContent {
    ReqwestFetchWebContent::with_lookup(WebFetchClientRecipe::default(), LOOPBACK, lookup)
}

fn refused(result: Result<FetchOutcome, FetchFailure>) -> String {
    match result {
        Err(FetchFailure::Refused(reason)) => reason,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[tokio::test]
async fn a_private_address_behind_a_discovered_slash_96_prefix_is_refused() {
    let adapter = adapter(dns64_slash96);
    let literal = slash96().synthesize(Ipv4Addr::new(10, 0, 0, 1));
    assert_eq!(
        refused(
            adapter
                .fetch(&request(&format!("http://[{literal}]/")))
                .await
        ),
        format!("10.0.0.1 (via {literal}) is not a public address")
    );
    assert_eq!(adapter.nat64.known().as_ref(), [slash96()], "discovered");
    let answer = slash96().synthesize(Ipv4Addr::new(10, 0, 0, 1));
    assert_eq!(
        refused(adapter.fetch(&request("http://private.test/")).await),
        format!(
            "private.test resolves to no public address: 10.0.0.1 (via {answer}) is not a public address"
        )
    );
}

#[tokio::test]
async fn a_private_address_behind_a_discovered_slash_64_prefix_is_refused() {
    let adapter = adapter(dns64_slash64);
    let literal = slash64().synthesize(Ipv4Addr::new(169, 254, 169, 254));
    assert_eq!(
        literal,
        "2001:4860:64:64:a9:fea9:fe00:0"
            .parse::<Ipv6Addr>()
            .unwrap()
    );
    assert_eq!(
        refused(
            adapter
                .fetch(&request(&format!("http://[{literal}]/")))
                .await
        ),
        format!("169.254.169.254 (via {literal}) is not a public address")
    );
    assert_eq!(adapter.nat64.known().as_ref(), [slash64()]);
    assert!(
        refused(adapter.fetch(&request("http://metadata.test/")).await)
            .contains("169.254.169.254 (via 2001:4860:64:64:a9:fea9:fe00:0)")
    );
}

/// A redirect hop to an address behind the prefix is refused before it is
/// requested.
#[tokio::test]
async fn a_redirect_behind_a_discovered_prefix_is_refused() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let hop = slash96().synthesize(Ipv4Addr::new(10, 0, 0, 1));
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buffer = [0; 4096];
        let _ = socket.read(&mut buffer).await;
        let response = format!(
            "HTTP/1.1 302 Found\r\nConnection: close\r\nLocation: http://[{hop}]/\r\nContent-Length: 0\r\n\r\n"
        );
        let _ = socket.write_all(response.as_bytes()).await;
    });
    assert_eq!(
        refused(
            adapter(dns64_slash96)
                .fetch(&request(&format!("http://first.test:{port}/")))
                .await
        ),
        format!("the redirect to http://[{hop}]/: 10.0.0.1 (via {hop}) is not a public address")
    );
    task.abort();
}

/// An address behind the prefix that embeds a public IPv4 address is
/// admitted: the literal, a DNS answer and a hop alike (checked without
/// connecting, since the address is not local).
#[tokio::test]
async fn a_public_address_behind_a_discovered_prefix_is_allowed() {
    let adapter = adapter(dns64_slash96);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    let prefixes = adapter.nat64.prefixes(deadline).await;
    assert_eq!(prefixes.as_ref(), [slash96()]);
    let public = slash96().synthesize(Ipv4Addr::new(8, 8, 8, 8));
    let url = url::Url::parse(&format!("http://[{public}]/")).unwrap();
    assert_eq!(authorize_url(&url, adapter.policy, &prefixes), Ok(()));
    assert_eq!(authorize_hop(&url, adapter.policy, &prefixes), Ok(()));
    assert_eq!(
        authorized_answers(
            "public.test",
            vec![answer(public)],
            LOOPBACK.address,
            &prefixes
        ),
        Ok(vec![answer(public)])
    );
}

/// No AAAA answer for `ipv4only.arpa` (the ordinary, non-NAT64 host): no
/// prefix, the result is kept, and addresses are judged as before.
#[tokio::test]
async fn no_discovery_answer_means_no_prefix_and_unchanged_judgement() {
    static LOOKUPS: AtomicUsize = AtomicUsize::new(0);
    fn ipv4_only(host: String) -> Answer {
        if host == DISCOVERY_NAME {
            LOOKUPS.fetch_add(1, Ordering::SeqCst);
        }
        Box::pin(async {
            Ok(vec![
                SocketAddr::from(([192, 0, 0, 170], 0)),
                SocketAddr::from(([127, 0, 0, 1], 0)),
            ])
        })
    }
    let adapter = adapter(ipv4_only);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    assert!(adapter.nat64.prefixes(deadline).await.is_empty());
    assert!(adapter.nat64.prefixes(deadline).await.is_empty());
    assert_eq!(LOOKUPS.load(Ordering::SeqCst), 1, "looked up once, kept");
    // The address is judged as ordinary allocated IPv6, as before #1942's
    // NAT64 discovery.
    let under = slash96().synthesize(Ipv4Addr::new(10, 0, 0, 1));
    let url = url::Url::parse(&format!("http://[{under}]/")).unwrap();
    assert_eq!(
        authorize_url(&url, DestinationPolicy::PRODUCTION, &[]),
        Ok(())
    );
}

/// A discovery that does not answer in time means no prefix for this
/// fetch, the fetch goes on, and the next fetch asks again.
#[tokio::test]
async fn a_discovery_timeout_does_not_block_the_fetch() {
    static LOOKUPS: AtomicUsize = AtomicUsize::new(0);
    fn hanging_discovery(host: String) -> Answer {
        match host.as_str() {
            DISCOVERY_NAME => {
                LOOKUPS.fetch_add(1, Ordering::SeqCst);
                Box::pin(async {
                    sleep(Duration::from_secs(30)).await;
                    Ok(Vec::new())
                })
            }
            _ => Box::pin(async { Ok(vec![SocketAddr::from(([127, 0, 0, 1], 0))]) }),
        }
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = [0; 4096];
            let _ = socket.read(&mut buffer).await;
            let _ = socket
                .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 2\r\n\r\nok")
                .await;
        }
    });
    let adapter = ReqwestFetchWebContent::with_discovery(
        WebFetchClientRecipe::default(),
        LOOPBACK,
        hanging_discovery,
        Duration::from_millis(100),
    );
    for _ in 0..2 {
        let started = Instant::now();
        let result = adapter
            .fetch(&request(&format!("http://local.test:{port}/")))
            .await;
        assert_eq!(
            result,
            Ok(FetchOutcome::SuccessBody {
                body: b"ok".to_vec(),
                content_type: None
            })
        );
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "{:?}",
            started.elapsed()
        );
    }
    assert_eq!(LOOKUPS.load(Ordering::SeqCst), 2, "a timeout is not kept");
    assert!(adapter.nat64.known().is_empty());
    // A discovery longer than the fetch's own deadline is cut by it.
    let adapter = ReqwestFetchWebContent::with_discovery(
        WebFetchClientRecipe::default(),
        LOOPBACK,
        hanging_discovery,
        Duration::from_secs(5),
    )
    .with_timeout(Duration::from_millis(200));
    let started = Instant::now();
    let result = adapter
        .fetch(&request(&format!("http://local.test:{port}/")))
        .await;
    assert_eq!(result, Err(FetchFailure::TimedOut));
    assert!(
        started.elapsed() < Duration::from_millis(600),
        "{:?}",
        started.elapsed()
    );
    task.abort();
}

/// A lookup error is not kept either: the next fetch asks again.
#[tokio::test]
async fn a_discovery_error_means_no_prefix_and_is_asked_again() {
    static LOOKUPS: AtomicUsize = AtomicUsize::new(0);
    fn failing_discovery(host: String) -> Answer {
        match host.as_str() {
            DISCOVERY_NAME => {
                LOOKUPS.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { Err(std::io::Error::other("no such name")) })
            }
            _ => Box::pin(async { Ok(vec![SocketAddr::from(([127, 0, 0, 1], 0))]) }),
        }
    }
    let adapter = adapter(failing_discovery);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    assert!(adapter.nat64.prefixes(deadline).await.is_empty());
    assert!(adapter.nat64.prefixes(deadline).await.is_empty());
    assert_eq!(LOOKUPS.load(Ordering::SeqCst), 2);
}
