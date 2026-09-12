use super::*;
use crate::application::agent_turn::use_cases::web_fetch::ParsedHttpUrl;
use std::sync::Arc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Barrier,
    time::{Duration, timeout},
};

async fn server(response: &'static [u8]) -> (String, tokio::task::JoinHandle<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = vec![0; 4096];
        let read = socket.read(&mut request).await.unwrap();
        request.truncate(read);
        socket.write_all(response).await.unwrap();
        request
    });
    (
        format!(
            "http://localtest.me:{address_port}/resource",
            address_port = address.port()
        ),
        task,
    )
}
fn request(url: &str) -> FetchRequest {
    FetchRequest {
        url: ParsedHttpUrl::parse(url).unwrap(),
    }
}

#[tokio::test]
async fn adapter_sends_get_and_version_user_agent() {
    let (url, peer) = server(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok").await;
    let result = ReqwestFetchWebContent::new(reqwest::Client::new())
        .fetch(&request(&url))
        .await
        .unwrap();
    assert_eq!(result, FetchOutcome::SuccessBody(b"ok".to_vec()));
    let wire = String::from_utf8_lossy(&peer.await.unwrap()).to_ascii_lowercase();
    assert!(wire.starts_with("get /resource http/1.1"));
    assert!(wire.contains(&format!("user-agent: quecto/{}", env!("CARGO_PKG_VERSION"))));
}

#[tokio::test]
async fn non_success_returns_without_polling_stalled_oversized_or_broken_body() {
    for headers in [
        "HTTP/1.1 404 Not Found\r\nTransfer-Encoding: chunked\r\n\r\n",
        "HTTP/1.1 500 Nope\r\nContent-Length: 5242881\r\n\r\n",
        "HTTP/1.1 429 Nope\r\nContent-Length: 10\r\n\r\nx",
    ] {
        let owned: &'static [u8] = Box::leak(headers.as_bytes().to_vec().into_boxed_slice());
        let (url, peer) = server(owned).await;
        let result = timeout(
            Duration::from_secs(1),
            ReqwestFetchWebContent::new(reqwest::Client::new()).fetch(&request(&url)),
        )
        .await
        .expect("status must be immediate")
        .unwrap();
        assert!(matches!(result, FetchOutcome::NonSuccessStatus(_)));
        peer.abort();
    }
}

#[tokio::test]
async fn success_caps_known_and_streamed_bodies_and_reports_read_errors() {
    let (url, _) = server(b"HTTP/1.1 200 OK\r\nContent-Length: 5242881\r\n\r\n").await;
    assert!(matches!(
        ReqwestFetchWebContent::new(reqwest::Client::new())
            .fetch(&request(&url))
            .await,
        Err(FetchFailure::TooLarge {
            actual_bytes: Some(5242881),
            ..
        })
    ));
    let (url, _) =
        server(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\nZ\r\n").await;
    assert!(matches!(
        ReqwestFetchWebContent::new(reqwest::Client::new())
            .fetch(&request(&url))
            .await,
        Err(FetchFailure::Read(_))
    ));
}

#[tokio::test]
async fn adapter_follows_redirects_and_reports_final_status() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let peer = tokio::spawn(async move {
        for response in [
            b"HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\n\r\n".as_slice(),
            b"HTTP/1.1 418 Nope\r\nTransfer-Encoding: chunked\r\n\r\n".as_slice(),
        ] {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 1024];
            let _ = socket.read(&mut request).await.unwrap();
            socket.write_all(response).await.unwrap();
        }
    });
    let result = ReqwestFetchWebContent::new(reqwest::Client::new())
        .fetch(&request(&format!("http://localtest.me:{port}/start")))
        .await
        .unwrap();
    assert_eq!(
        result,
        FetchOutcome::NonSuccessStatus(HttpStatus::new(418, Some("I'm a teapot".into())))
    );
    peer.abort();
}

#[tokio::test]
async fn dropping_fetch_cancels_a_stalled_request() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let accepted = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0; 1024];
        let _ = socket.read(&mut request).await.unwrap();
        timeout(Duration::from_secs(1), socket.read(&mut request)).await
    });
    let adapter = ReqwestFetchWebContent::new(reqwest::Client::new());
    let req = request(&format!("http://localtest.me:{port}/stall"));
    let task = tokio::spawn(async move { adapter.fetch(&req).await });
    tokio::time::sleep(Duration::from_millis(50)).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(
        accepted.await.unwrap().is_ok(),
        "peer must observe prompt connection close"
    );
}

#[tokio::test]
async fn adapter_allows_concurrent_requests() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let barrier = Arc::new(Barrier::new(3));
    let serve = tokio::spawn({
        let barrier = barrier.clone();
        async move {
            for body in [b"a", b"b"] {
                let (mut s, _) = listener.accept().await.unwrap();
                let barrier = barrier.clone();
                tokio::spawn(async move {
                    let mut buf = [0; 1024];
                    let _ = s.read(&mut buf).await;
                    barrier.wait().await;
                    s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\n")
                        .await
                        .unwrap();
                    s.write_all(body).await.unwrap();
                });
            }
        }
    });
    let adapter = ReqwestFetchWebContent::new(reqwest::Client::new());
    let r1 = request(&format!("http://localtest.me:{port}/1"));
    let r2 = request(&format!("http://localtest.me:{port}/2"));
    let (a, b, _) = tokio::join!(adapter.fetch(&r1), adapter.fetch(&r2), barrier.wait());
    assert!(matches!(a, Ok(FetchOutcome::SuccessBody(_))));
    assert!(matches!(b, Ok(FetchOutcome::SuccessBody(_))));
    serve.await.unwrap();
}
