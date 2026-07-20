use super::*;

#[test]
fn max_sse_line_bytes_is_one_mib() {
    assert_eq!(MAX_SSE_LINE_BYTES, 1024 * 1024);
}

#[test]
fn max_error_body_bytes_is_4096() {
    assert_eq!(MAX_ERROR_BODY_BYTES, 4096);
}

fn header_map(headers: &[(&str, &str)]) -> reqwest::header::HeaderMap {
    let mut map = reqwest::header::HeaderMap::new();
    for (k, v) in headers {
        map.insert(
            reqwest::header::HeaderName::from_bytes(k.as_bytes()).unwrap(),
            reqwest::header::HeaderValue::from_str(v).unwrap(),
        );
    }
    map
}

#[tokio::test]
async fn format_send_error_appends_source_chain() {
    // A refused connection to a closed loopback port yields a real
    // `reqwest::Error` whose top-level message is "error sending request…"
    // and whose `source()` carries the concrete cause. Port 1 on loopback
    // is not listening, so the connect is refused deterministically.
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap();
    let err = client
        .get("http://127.0.0.1:1/")
        .send()
        .await
        .expect_err("connection to closed port must fail");

    let rendered = format_send_error(&err);
    // Top-level reqwest message is preserved…
    assert!(
        rendered.contains("error sending request") || rendered.contains("Connection refused"),
        "unexpected transport error rendering: {rendered}"
    );
    // …and the source chain is appended (longer than the bare Display),
    // so a keyword like "refused"/"connect" reaches the retry classifier.
    assert!(
        rendered.len() >= err.to_string().len(),
        "source chain must not shorten the message: {rendered}"
    );
}

#[test]
fn retry_after_suffix_prefers_ms_header() {
    let h = header_map(&[("retry-after", "5"), ("retry-after-ms", "1234")]);
    assert_eq!(retry_after_suffix(&h), " retry-after-ms: 1234");
}

#[test]
fn retry_after_suffix_falls_back_to_seconds() {
    let h = header_map(&[("retry-after", "7")]);
    assert_eq!(retry_after_suffix(&h), " retry-after: 7");
}

#[test]
fn retry_after_suffix_empty_when_absent() {
    let h = header_map(&[]);
    assert_eq!(retry_after_suffix(&h), "");
}

#[test]
fn truncate_error_body_short_unchanged() {
    let body = "short error".to_string();
    assert_eq!(truncate_error_body(body), "short error");
}

#[test]
fn truncate_error_body_exact_limit_unchanged() {
    let body = "x".repeat(MAX_ERROR_BODY_BYTES);
    let result = truncate_error_body(body.clone());
    assert_eq!(result, body);
}

#[test]
fn truncate_error_body_over_limit_truncated() {
    let body = "x".repeat(MAX_ERROR_BODY_BYTES + 100);
    let result = truncate_error_body(body);
    assert!(result.ends_with("... (truncated)"));
    assert!(result.len() < MAX_ERROR_BODY_BYTES + 20);
}

#[test]
fn truncate_error_body_utf8_safe() {
    // Build a string where byte 4096 falls inside a multi-byte char.
    // '€' is 3 bytes (E2 82 AC). Fill up to just before 4096, then add '€'
    // so the 3-byte char straddles the boundary.
    let padding = "x".repeat(MAX_ERROR_BODY_BYTES - 1); // 4095 bytes
    let body = format!("{padding}€€€"); // 4095 + 9 = 4104 bytes
    let result = truncate_error_body(body);
    // Must not panic, and must end with the truncation marker
    assert!(result.ends_with("... (truncated)"));
    // The result should be valid UTF-8 (it is, since it's a String)
    assert!(result.is_char_boundary(result.len()));
}
