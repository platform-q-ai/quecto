//! #2248 review L5: the causes named by their message are named from the
//! messages the dependencies really produce, so a change in their wording
//! fails here rather than silently dropping the plain reason. Local peers
//! only: nothing leaves the machine.
use super::*;
use crate::application::agent_turn::use_cases::web_fetch::ParsedHttpUrl;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use tokio_rustls::rustls;

fn request(url: &str) -> FetchRequest {
    FetchRequest {
        url: ParsedHttpUrl::parse(url).unwrap(),
    }
}

fn transport(result: Result<FetchOutcome, FetchFailure>) -> String {
    match result {
        Err(FetchFailure::Transport(detail)) => detail,
        other => panic!("not a transport failure: {other:?}"),
    }
}

/// A TLS peer for [`TEST_HOST`] offering only the application protocol
/// `alpn`, when given.
fn tls_acceptor(alpn: Option<&[u8]>) -> tokio_rustls::TlsAcceptor {
    use rustls::{ServerConfig, pki_types::PrivateKeyDer};
    let _ = rustls::crypto::ring::default_provider().install_default();
    let certified = rcgen::generate_simple_self_signed(vec![TEST_HOST.into()]).unwrap();
    let key = PrivateKeyDer::try_from(certified.key_pair.serialize_der()).unwrap();
    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![certified.cert.der().clone()], key)
        .unwrap();
    config.alpn_protocols = alpn.map(|alpn| vec![alpn.to_vec()]).unwrap_or_default();
    tokio_rustls::TlsAcceptor::from(Arc::new(config))
}

/// The detail of a fetch of `https://TEST_HOST` from a peer that `serve`s
/// its one connection.
async fn fetched_from<F, Fut>(serve: F) -> String
where
    F: FnOnce(tokio::net::TcpStream) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send,
{
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let peer = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        serve(socket).await;
    });
    let detail = transport(
        ReqwestFetchWebContent::with_policy(LOOPBACK_FOR_TESTS)
            .fetch(&request(&format!("https://{TEST_HOST}:{port}/")))
            .await,
    );
    peer.abort();
    detail
}

/// A peer that shares no application protocol with the client ends the
/// handshake with a fatal alert, as rustls says it.
#[tokio::test]
async fn a_fatal_alert_from_rustls_is_a_failed_handshake() {
    let tls = tls_acceptor(Some(b"not-http"));
    let detail = fetched_from(|socket| async move {
        let _ = tls.accept(socket).await;
    })
    .await;
    assert!(detail.starts_with("TLS handshake failed: "), "{detail}");
    assert!(detail.contains("received fatal alert"), "{detail}");
}

/// A peer that answers TLS with plain HTTP sends what rustls calls a
/// corrupt message.
#[tokio::test]
async fn a_plain_text_answer_to_tls_is_a_failed_handshake() {
    let detail = fetched_from(|mut socket| async move {
        let mut buffer = [0; 4096];
        let _ = socket.read(&mut buffer).await;
        let _ = socket
            .write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n")
            .await;
    })
    .await;
    assert!(detail.starts_with("TLS handshake failed: "), "{detail}");
    assert!(detail.contains("received corrupt message"), "{detail}");
}

/// Every marker is in a message rustls's own error values display, or
/// (`dns error`) in hyper-util's, which only it constructs and which
/// `a_failed_lookup_says_so` reads through a real fetch.
#[test]
fn every_message_marker_is_in_a_dependency_s_own_message() {
    use rustls::{AlertDescription, CertificateError, Error as Tls, InvalidMessage};
    let produced = [
        Tls::InvalidCertificate(CertificateError::UnknownIssuer).to_string(),
        Tls::AlertReceived(AlertDescription::HandshakeFailure).to_string(),
        Tls::InvalidMessage(InvalidMessage::InvalidContentType).to_string(),
    ];
    for (marker, _) in failure_detail::MESSAGE_REASONS {
        let through_a_real_fetch = *marker == "dns error";
        assert!(
            through_a_real_fetch || produced.iter().any(|message| message.contains(marker)),
            "{marker}: {produced:?}"
        );
    }
}
