use super::*;

#[tokio::test]
async fn test_ssrf_restricted_urls() {
    let cases = [
        ("http://localhost/secret", "localhost"),
        ("http://127.0.0.1/secret", "loopback IP"),
        ("http://169.254.169.254/latest/meta-data/", "AWS metadata"),
        ("http://10.0.0.1/internal", "private RFC1918"),
        ("http://[::1]/secret", "IPv6 loopback"),
        (
            "http://metadata.google.internal/computeMetadata/v1/",
            "Google metadata",
        ),
    ];

    let tool = WebFetchTool::new(32);
    for (url, label) in cases {
        let result = tool
            .execute(&format!(r#"{{"url":"{url}"}}"#))
            .await
            .unwrap_or_else(|e| panic!("{label}: tool execution failed: {e}"));
        assert!(result.is_error, "{label}: request should be rejected");
        assert!(
            result.content.contains("restricted"),
            "{label}: expected 'restricted' in error, got: {}",
            result.content
        );
    }
}
