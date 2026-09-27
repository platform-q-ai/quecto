//! #1942 ledger rows 3, 6 and 10 in the execute path: one deadline covers
//! DNS, connect, every redirect and the body; the recipe's connect timeout
//! is applied; and a mixed DNS answer never connects its denied address,
//! whether the fetch is cancelled or runs beside others. Loopback only: a
//! hanging connect comes from a listener whose accept queue is full.
use super::*;
use crate::application::agent_turn::use_cases::web_fetch::ParsedHttpUrl;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Instant;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpSocket, TcpStream},
    time::{Duration, sleep},
};

fn request(url: &str) -> FetchRequest {
    FetchRequest {
        url: ParsedHttpUrl::parse(url).unwrap(),
    }
}

fn any_name(_: &str) -> Result<(), String> {
    Ok(())
}

/// Only 127.0.0.2 beyond public space.
fn second_loopback_only(address: IpAddr) -> Result<(), NonPublicAddress> {
    match address {
        IpAddr::V4(v4) if v4 == std::net::Ipv4Addr::new(127, 0, 0, 2) => Ok(()),
        _ => authorize_destination(address),
    }
}

const SECOND_LOOPBACK: DestinationPolicy = DestinationPolicy {
    address: second_loopback_only,
    name: any_name,
};

const LOOPBACK: DestinationPolicy = DestinationPolicy {
    address: loopback_or_public,
    name: any_name,
};

type Answer = Pin<Box<dyn Future<Output = std::io::Result<Vec<SocketAddr>>> + Send>>;

/// Every name: 127.0.0.1, at once.
fn loopback_now(_: String) -> Answer {
    Box::pin(async { Ok(vec![SocketAddr::from(([127, 0, 0, 1], 0))]) })
}

/// Every name: 127.0.0.1, after 400 ms.
fn loopback_after_400ms(_: String) -> Answer {
    Box::pin(async {
        sleep(Duration::from_millis(400)).await;
        Ok(vec![SocketAddr::from(([127, 0, 0, 1], 0))])
    })
}

/// Every name: 127.0.0.1, after 120 ms.
fn loopback_after_120ms(_: String) -> Answer {
    Box::pin(async {
        sleep(Duration::from_millis(120)).await;
        Ok(vec![SocketAddr::from(([127, 0, 0, 1], 0))])
    })
}

/// Every name: the refused 127.0.0.1 first, then the allowed 127.0.0.2.
fn refused_then_allowed(_: String) -> Answer {
    Box::pin(async {
        Ok(vec![
            SocketAddr::from(([127, 0, 0, 1], 0)),
            SocketAddr::from(([127, 0, 0, 2], 0)),
        ])
    })
}

fn adapter(
    recipe: WebFetchClientRecipe,
    policy: DestinationPolicy,
    lookup: Lookup,
) -> ReqwestFetchWebContent {
    ReqwestFetchWebContent::with_lookup(recipe, policy, lookup)
}

/// How a peer answers each connection it accepts.
#[derive(Clone, Copy)]
enum Answering {
    /// A whole 200 `ok` at once.
    Ok,
    /// Reads the request and never answers.
    Stall,
}

/// Serves `listener`, counting accepted connections.
fn serve(
    listener: TcpListener,
    answering: Answering,
) -> (Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let accepted = Arc::new(AtomicUsize::new(0));
    let task = tokio::spawn({
        let accepted = accepted.clone();
        async move {
            let mut held = Vec::new();
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                accepted.fetch_add(1, Ordering::SeqCst);
                let mut buffer = [0; 4096];
                let _ = socket.read(&mut buffer).await;
                match answering {
                    Answering::Ok => {
                        let _ = socket
                            .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 2\r\n\r\nok")
                            .await;
                    }
                    Answering::Stall => held.push(socket),
                }
            }
        }
    });
    (accepted, task)
}

/// Listeners on 127.0.0.1 (the denied address) and 127.0.0.2 (the allowed
/// one) sharing a port: the resolver's answers carry no port.
async fn twin(
    allowed: Answering,
) -> (
    u16,
    Arc<AtomicUsize>,
    Arc<AtomicUsize>,
    [tokio::task::JoinHandle<()>; 2],
) {
    loop {
        let denied = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = denied.local_addr().unwrap().port();
        let Ok(second) = TcpListener::bind(("127.0.0.2", port)).await else {
            continue;
        };
        let (denied_accepted, denied_task) = serve(denied, Answering::Ok);
        let (allowed_accepted, allowed_task) = serve(second, allowed);
        return (
            port,
            denied_accepted,
            allowed_accepted,
            [denied_task, allowed_task],
        );
    }
}

/// A loopback listener whose accept queue is full: a new connection's SYN
/// is dropped, so connecting to it hangs. The returned values keep it so.
async fn hanging_listener() -> (u16, TcpListener, Vec<TcpStream>) {
    let socket = TcpSocket::new_v4().unwrap();
    socket.bind(SocketAddr::from(([127, 0, 0, 1], 0))).unwrap();
    let listener = socket.listen(0).unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut fillers = Vec::new();
    loop {
        match tokio::time::timeout(
            Duration::from_millis(200),
            TcpStream::connect(("127.0.0.1", port)),
        )
        .await
        {
            Ok(Ok(stream)) => fillers.push(stream),
            _ => return (port, listener, fillers),
        }
    }
}

fn elapsed_within(started: Instant, low: u64, high: u64) {
    let elapsed = started.elapsed();
    assert!(
        elapsed >= Duration::from_millis(low) && elapsed < Duration::from_millis(high),
        "{elapsed:?} not in {low}..{high} ms"
    );
}

/// Row 3: a slow DNS answer counts against the one deadline; nothing is
/// connected.
#[tokio::test]
async fn a_slow_dns_answer_counts_against_the_one_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (accepted, task) = serve(listener, Answering::Ok);
    let started = Instant::now();
    let result = adapter(
        WebFetchClientRecipe::default(),
        LOOPBACK,
        loopback_after_400ms,
    )
    .with_timeout(Duration::from_millis(250))
    .fetch(&request(&format!("http://slow-dns.test:{port}/")))
    .await;
    assert_eq!(result, Err(FetchFailure::TimedOut));
    elapsed_within(started, 250, 390);
    assert_eq!(accepted.load(Ordering::SeqCst), 0);
    task.abort();
}

/// Row 3: a connect that hangs counts against the one deadline.
#[tokio::test]
async fn a_hanging_connect_counts_against_the_one_deadline() {
    let (port, _listener, _fillers) = hanging_listener().await;
    let started = Instant::now();
    let result = adapter(WebFetchClientRecipe::default(), LOOPBACK, loopback_now)
        .with_timeout(Duration::from_millis(300))
        .fetch(&request(&format!("http://hang.test:{port}/")))
        .await;
    assert_eq!(result, Err(FetchFailure::TimedOut));
    elapsed_within(started, 300, 800);
}

/// Row 10: the recipe's connect timeout is applied: the hanging connect
/// fails at 150 ms, long before the three-second deadline.
#[tokio::test]
async fn the_recipe_connect_timeout_is_applied() {
    let (port, _listener, _fillers) = hanging_listener().await;
    let recipe = WebFetchClientRecipe::default().connect_timeout(Duration::from_millis(150));
    let started = Instant::now();
    let result = adapter(recipe, LOOPBACK, loopback_now)
        .with_timeout(Duration::from_secs(3))
        .fetch(&request(&format!("http://hang.test:{port}/")))
        .await;
    assert_eq!(result, Err(FetchFailure::TimedOut));
    elapsed_within(started, 150, 1000);
}

/// Serves `/start` as a redirect to `/final` after 120 ms, and `/final` as
/// a 200 whose body comes 150 ms after its head.
async fn slow_redirect_then_slow_body() -> (u16, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut buffer = [0; 4096];
                let read = socket.read(&mut buffer).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buffer[..read]).to_string();
                if head.starts_with("GET /start") {
                    sleep(Duration::from_millis(120)).await;
                    let _ = socket
                        .write_all(b"HTTP/1.1 302 Found\r\nConnection: close\r\nLocation: /final\r\nContent-Length: 0\r\n\r\n")
                        .await;
                } else {
                    let _ = socket
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 4\r\n\r\n",
                        )
                        .await;
                    sleep(Duration::from_millis(150)).await;
                    let _ = socket.write_all(b"done").await;
                }
            });
        }
    });
    (port, task)
}

/// Row 3: DNS (120 ms, twice), the redirect (120 ms) and the body (150 ms)
/// each fit a 400 ms deadline, but together they do not: the one deadline
/// spans them all. The same fetch fits a two-second deadline.
#[tokio::test]
async fn one_deadline_spans_dns_connect_redirect_and_body() {
    let (port, task) = slow_redirect_then_slow_body().await;
    let url = format!("http://slow.test:{port}/start");
    let started = Instant::now();
    let result = adapter(
        WebFetchClientRecipe::default(),
        LOOPBACK,
        loopback_after_120ms,
    )
    .with_timeout(Duration::from_millis(400))
    .fetch(&request(&url))
    .await;
    assert!(
        matches!(result, Err(FetchFailure::TimedOut | FetchFailure::Read(_))),
        "{result:?}"
    );
    elapsed_within(started, 400, 600);
    let result = adapter(
        WebFetchClientRecipe::default(),
        LOOPBACK,
        loopback_after_120ms,
    )
    .with_timeout(Duration::from_secs(2))
    .fetch(&request(&url))
    .await;
    assert_eq!(
        result,
        Ok(FetchOutcome::SuccessBody {
            body: b"done".to_vec(),
            content_type: None
        })
    );
    task.abort();
}

/// Row 3: a body that stalls after its head is cut at the deadline.
#[tokio::test]
async fn a_stalled_body_is_cut_at_the_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buffer = [0; 4096];
        let _ = socket.read(&mut buffer).await;
        let _ = socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\npa")
            .await;
        sleep(Duration::from_secs(30)).await;
    });
    let started = Instant::now();
    let result = adapter(WebFetchClientRecipe::default(), LOOPBACK, loopback_now)
        .with_timeout(Duration::from_millis(300))
        .fetch(&request(&format!("http://stall.test:{port}/")))
        .await;
    assert!(matches!(result, Err(FetchFailure::Read(_))), "{result:?}");
    elapsed_within(started, 300, 800);
    task.abort();
}

/// Row 6 (a): a fetch of a mixed answer, cancelled while it waits on the
/// allowed address, never connected the denied one.
#[tokio::test]
async fn a_cancelled_fetch_of_a_mixed_answer_never_connects_the_denied_address() {
    let (port, denied, allowed, tasks) = twin(Answering::Stall).await;
    let fetch = tokio::spawn(async move {
        adapter(
            WebFetchClientRecipe::default(),
            SECOND_LOOPBACK,
            refused_then_allowed,
        )
        .fetch(&request(&format!("http://mixed.test:{port}/")))
        .await
    });
    let waited = Instant::now();
    while allowed.load(Ordering::SeqCst) == 0 && waited.elapsed() < Duration::from_secs(5) {
        sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        allowed.load(Ordering::SeqCst),
        1,
        "the allowed address was reached"
    );
    fetch.abort();
    assert!(fetch.await.unwrap_err().is_cancelled());
    sleep(Duration::from_millis(100)).await;
    assert_eq!(denied.load(Ordering::SeqCst), 0, "the denied address never");
    tasks.iter().for_each(|task| task.abort());
}

/// Row 6 (b): concurrent fetches of a mixed answer all reach only the
/// allowed address.
#[tokio::test]
async fn concurrent_fetches_of_a_mixed_answer_never_connect_the_denied_address() {
    let (port, denied, allowed, tasks) = twin(Answering::Ok).await;
    let adapter = adapter(
        WebFetchClientRecipe::default(),
        SECOND_LOOPBACK,
        refused_then_allowed,
    );
    let request = request(&format!("http://mixed.test:{port}/"));
    let results = futures::future::join_all((0..8).map(|_| adapter.fetch(&request))).await;
    for result in results {
        assert_eq!(
            result,
            Ok(FetchOutcome::SuccessBody {
                body: b"ok".to_vec(),
                content_type: None
            })
        );
    }
    assert_eq!(allowed.load(Ordering::SeqCst), 8);
    assert_eq!(denied.load(Ordering::SeqCst), 0);
    tasks.iter().for_each(|task| task.abort());
}
