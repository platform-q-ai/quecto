//! #2209: a transport or read failure says its cause, not only reqwest's
//! top line, and a failure on a redirect hop names the hop. Local
//! listeners and injected lookups only: nothing leaves the machine.
use super::*;
use crate::application::agent_turn::use_cases::web_fetch::ParsedHttpUrl;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

fn request(url: &str) -> FetchRequest {
    FetchRequest {
        url: ParsedHttpUrl::parse(url).unwrap(),
    }
}

fn adapter() -> ReqwestFetchWebContent {
    ReqwestFetchWebContent::with_policy(LOOPBACK_FOR_TESTS)
}

/// A local port nothing listens on.
async fn closed_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    listener.local_addr().unwrap().port()
}

/// A listener answering its first connection with `response`.
async fn answering(response: String) -> (u16, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buffer = [0; 4096];
        let _ = socket.read(&mut buffer).await;
        let _ = socket.write_all(response.as_bytes()).await;
    });
    (port, task)
}

fn transport(result: Result<FetchOutcome, FetchFailure>) -> String {
    match result {
        Err(FetchFailure::Transport(detail)) => detail,
        other => panic!("not a transport failure: {other:?}"),
    }
}

#[tokio::test]
async fn a_refused_connection_says_so() {
    let port = closed_port().await;
    let url = format!("http://localhost:{port}/x");
    let detail = transport(adapter().fetch(&request(&url)).await);
    assert!(
        detail.starts_with(&format!(
            "connection refused: error sending request for url ({url})"
        )),
        "{detail}"
    );
    assert!(detail.contains("Connection refused"), "{detail}");
    assert!(detail.len() <= failure_detail::MAX_DETAIL_BYTES, "{detail}");
}

/// The issue's E9c: a public URL redirecting to a dead one fails on the
/// hop, and says so, naming it.
#[tokio::test]
async fn a_failure_on_a_redirect_hop_names_the_hop() {
    let dead = closed_port().await;
    let hop = format!("http://localhost:{dead}/gone");
    let (port, peer) = answering(format!(
        "HTTP/1.1 302 Found\r\nConnection: close\r\nLocation: {hop}\r\nContent-Length: 0\r\n\r\n"
    ))
    .await;
    let detail = transport(
        adapter()
            .fetch(&request(&format!("http://localhost:{port}/start")))
            .await,
    );
    assert!(
        detail.starts_with(&format!(
            "connection refused on the redirect hop to {hop}: error sending request: "
        )),
        "{detail}"
    );
    peer.abort();
}

/// Credentials in the URL are never repeated in the failure.
#[tokio::test]
async fn credentials_are_not_repeated_in_a_failure() {
    let port = closed_port().await;
    let detail = transport(
        adapter()
            .fetch(&request(&format!("http://user:hunter2@localhost:{port}/")))
            .await,
    );
    assert!(
        !detail.contains("hunter2") && !detail.contains("user@"),
        "{detail}"
    );
    assert!(!detail.contains("redirect hop"), "{detail}");
}

fn failing_lookup(
    _host: String,
) -> Pin<Box<dyn Future<Output = std::io::Result<Vec<SocketAddr>>> + Send>> {
    Box::pin(async {
        Err(std::io::Error::other(
            "failed to lookup address information: Name or service not known",
        ))
    })
}

#[tokio::test]
async fn a_failed_lookup_says_so() {
    let adapter = ReqwestFetchWebContent::with_lookup(
        WebFetchClientRecipe::default(),
        LOOPBACK_FOR_TESTS,
        failing_lookup,
    );
    let detail = transport(adapter.fetch(&request("http://nowhere.example/")).await);
    assert!(detail.starts_with("DNS lookup failed: "), "{detail}");
    assert!(detail.contains("Name or service not known"), "{detail}");
}

#[tokio::test]
async fn an_untrusted_certificate_says_so() {
    use tokio_rustls::rustls::{ServerConfig, pki_types::PrivateKeyDer};
    let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
    let certified = rcgen::generate_simple_self_signed(vec![TEST_HOST.into()]).unwrap();
    let key = PrivateKeyDer::try_from(certified.key_pair.serialize_der()).unwrap();
    let tls = tokio_rustls::TlsAcceptor::from(Arc::new(
        ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![certified.cert.der().clone()], key)
            .unwrap(),
    ));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let peer = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let _ = tls.accept(socket).await;
    });
    let detail = transport(
        adapter()
            .fetch(&request(&format!("https://{TEST_HOST}:{port}/")))
            .await,
    );
    assert!(
        detail.starts_with("TLS certificate not accepted: "),
        "{detail}"
    );
    assert!(detail.contains("invalid peer certificate"), "{detail}");
    peer.abort();
}

/// A body that breaks off says why, and a read after a redirect names the
/// hop it was read from.
#[tokio::test]
async fn a_broken_body_says_why() {
    let broken = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\nZ\r\n";
    let (port, peer) = answering(broken.to_owned()).await;
    let url = format!("http://localhost:{port}/body");
    let detail = match adapter().fetch(&request(&url)).await {
        Err(FetchFailure::Read(detail)) => detail,
        other => panic!("not a read failure: {other:?}"),
    };
    assert!(detail.contains(&format!("for url ({url}): ")), "{detail}");
    assert!(detail.contains("Invalid chunk size"), "{detail}");
    peer.abort();

    let (target, target_peer) = answering(broken.to_owned()).await;
    let hop = format!("http://localhost:{target}/final");
    let (port, peer) = answering(format!(
        "HTTP/1.1 301 Moved\r\nConnection: close\r\nLocation: {hop}\r\nContent-Length: 0\r\n\r\n"
    ))
    .await;
    let detail = match adapter()
        .fetch(&request(&format!("http://localhost:{port}/")))
        .await
    {
        Err(FetchFailure::Read(detail)) => detail,
        other => panic!("not a read failure: {other:?}"),
    };
    assert!(
        detail.starts_with(&format!("failed on the redirect hop to {hop}: ")),
        "{detail}"
    );
    peer.abort();
    target_peer.abort();
}

/// A read failure names its URL once, from the detail, never with the
/// credentials reqwest's own message would repeat.
#[tokio::test]
async fn a_broken_body_names_its_url_once_without_credentials() {
    let broken = "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\nZ\r\n";
    let (port, peer) = answering(broken.to_owned()).await;
    let detail = match adapter()
        .fetch(&request(&format!(
            "http://user:hunter2@localhost:{port}/body"
        )))
        .await
    {
        Err(FetchFailure::Read(detail)) => detail,
        other => panic!("not a read failure: {other:?}"),
    };
    assert!(
        !detail.contains("hunter2") && !detail.contains("user@"),
        "{detail}"
    );
    assert_eq!(detail.matches("for url (").count(), 1, "{detail}");
    peer.abort();
}
