use super::*;

#[tokio::test]
async fn test_discard_error_body_bounded_does_not_wait_for_oversized_response() {
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
        stream
            .write_all(b"server-secret-detail-before-cap")
            .unwrap();
        stream.flush().unwrap();
        std::thread::sleep(std::time::Duration::from_secs(5));
    });

    let resp = reqwest::Client::new()
        .get(format!("http://{}", addr))
        .send()
        .await
        .unwrap();
    let start = std::time::Instant::now();
    discard_error_body_bounded(resp).await;

    assert!(
        start.elapsed() < std::time::Duration::from_secs(1),
        "bounded error handling should not wait for or buffer the oversized response body"
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

#[tokio::test]
async fn test_token_error_messages_omit_response_details() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(500).set_body_string("server-secret-detail"))
        .mount(&server)
        .await;

    let config = OAuthConfig::with_base_url(&server.uri());

    let errors = [
        exchange_anthropic_code(&config, "code#state", "verifier")
            .await
            .unwrap_err()
            .to_string(),
        refresh_anthropic_token(&config, "refresh")
            .await
            .unwrap_err()
            .to_string(),
        exchange_openai_code(&config, "code", "verifier")
            .await
            .unwrap_err()
            .to_string(),
        refresh_openai_token(&config, "refresh")
            .await
            .unwrap_err()
            .to_string(),
    ];

    for err in errors {
        assert!(err.contains("500"), "error should include status: {err}");
        assert!(
            !err.contains("server-secret-detail"),
            "OAuth token errors must not include response body details: {err}"
        );
    }
}
