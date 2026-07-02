use super::*;

#[tokio::test]
async fn test_read_error_body_bounded_stops_at_documented_cap() {
    use std::io::Write;
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0; 1024];
        let _ = std::io::Read::read(&mut stream, &mut request).unwrap();
        write!(
            stream,
            "HTTP/1.1 500 Internal Server Error\r\nContent-Length: {}\r\n\r\n",
            MAX_ERROR_BODY_BYTES * 4
        )
        .unwrap();
        stream.write_all(&vec![b'x'; MAX_ERROR_BODY_BYTES]).unwrap();
        stream.write_all(b"sentinel-after-cap").unwrap();
        stream.flush().unwrap();
        std::thread::sleep(std::time::Duration::from_secs(5));
    });

    let resp = reqwest::Client::new()
        .get(format!("http://{}", addr))
        .send()
        .await
        .unwrap();
    let start = std::time::Instant::now();
    let body = read_error_body_bounded(resp).await;

    assert_eq!(
        body.len(),
        MAX_ERROR_BODY_BYTES,
        "bounded error reader must retain at most the documented cap"
    );
    assert!(
        start.elapsed() < std::time::Duration::from_secs(1),
        "bounded error reader should stop after the capped prefix, not wait for the full body"
    );
    assert!(
        !String::from_utf8_lossy(&body).contains("sentinel-after-cap"),
        "bounded error reader must not consume bytes beyond the documented cap"
    );
    drop(handle);
}

#[tokio::test]
async fn test_device_code_error_message_omits_oversized_response_details() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let oversized_body = "server-secret-detail".repeat(MAX_ERROR_BODY_BYTES);

    Mock::given(method("POST"))
        .and(path("/device/code"))
        .respond_with(ResponseTemplate::new(500).set_body_string(oversized_body.clone()))
        .mount(&server)
        .await;

    let config = OAuthConfig::with_base_url(&server.uri());
    let err = request_device_code(&config).await.unwrap_err().to_string();

    assert!(
        err.contains("device code request failed (500)"),
        "error should report the failed status, got: {err}"
    );
    assert!(
        !err.contains("server-secret-detail"),
        "device code errors must not include response body details"
    );
}
