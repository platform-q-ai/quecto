// Health server: HTTP /health and /ready endpoints for observability.
//
// Uses raw tokio TCP (no hyper/axum dependency) for minimal binary footprint.
// Provides /health (liveness) and /ready (readiness) endpoints.

use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Readiness checker: determines whether the system has usable providers.
pub trait ReadinessCheck: Send + Sync {
    /// Returns true if at least one LLM provider is available.
    fn is_ready(&self) -> bool;
}

/// A simple readiness check backed by a boolean flag.
#[derive(Debug)]
pub struct StaticReadiness {
    ready: std::sync::atomic::AtomicBool,
}

impl StaticReadiness {
    pub fn new(ready: bool) -> Self {
        Self {
            ready: std::sync::atomic::AtomicBool::new(ready),
        }
    }

    pub fn set_ready(&self, ready: bool) {
        self.ready
            .store(ready, std::sync::atomic::Ordering::Relaxed);
    }
}

impl ReadinessCheck for StaticReadiness {
    fn is_ready(&self) -> bool {
        self.ready.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// Lightweight HTTP health server.
pub struct HealthServer {
    listener: TcpListener,
    readiness: Arc<dyn ReadinessCheck>,
}

impl HealthServer {
    /// Bind to the given address and create a health server.
    pub async fn bind(
        addr: &str,
        readiness: Arc<dyn ReadinessCheck>,
    ) -> Result<Self, std::io::Error> {
        let listener = TcpListener::bind(addr).await?;
        Ok(Self {
            listener,
            readiness,
        })
    }

    /// Returns the local address the server is listening on.
    pub fn local_addr(&self) -> Result<std::net::SocketAddr, std::io::Error> {
        self.listener.local_addr()
    }

    /// Run the server loop, accepting connections until cancelled.
    pub async fn run(&self) {
        loop {
            match self.listener.accept().await {
                Ok((stream, _)) => {
                    let readiness = self.readiness.clone();
                    tokio::spawn(async move {
                        handle_connection(stream, &*readiness).await;
                    });
                }
                Err(e) => {
                    tracing::error!(error = %e, "health server accept error");
                }
            }
        }
    }
}

/// Handle a single HTTP connection.
async fn handle_connection(mut stream: tokio::net::TcpStream, readiness: &dyn ReadinessCheck) {
    let mut buf = [0u8; 1024];
    let n = match stream.read(&mut buf).await {
        Ok(n) if n > 0 => n,
        _ => return,
    };

    let request = String::from_utf8_lossy(&buf[..n]);
    let (status, body) = route_request(&request, readiness);

    let response = format!(
        "HTTP/1.1 {status}\r\n\
         Content-Type: application/json\r\n\
         Content-Length: {len}\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        status = status,
        len = body.len(),
        body = body,
    );

    let _ = stream.write_all(response.as_bytes()).await;
}

/// Route an HTTP request to the appropriate handler.
fn route_request(request: &str, readiness: &dyn ReadinessCheck) -> (&'static str, String) {
    let path = parse_request_path(request);

    match path {
        "/health" => ("200 OK", r#"{"status":"ok"}"#.to_string()),
        "/ready" => {
            if readiness.is_ready() {
                ("200 OK", r#"{"ready":true}"#.to_string())
            } else {
                ("503 Service Unavailable", r#"{"ready":false}"#.to_string())
            }
        }
        _ => ("404 Not Found", r#"{"error":"not found"}"#.to_string()),
    }
}

/// Extract the path from an HTTP request line (e.g. "GET /health HTTP/1.1").
fn parse_request_path(request: &str) -> &str {
    request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_request_path_health() {
        let req = "GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n";
        assert_eq!(parse_request_path(req), "/health");
    }

    #[test]
    fn test_parse_request_path_ready() {
        let req = "GET /ready HTTP/1.1\r\nHost: localhost\r\n\r\n";
        assert_eq!(parse_request_path(req), "/ready");
    }

    #[test]
    fn test_parse_request_path_unknown() {
        let req = "GET /metrics HTTP/1.1\r\n\r\n";
        assert_eq!(parse_request_path(req), "/metrics");
    }

    #[test]
    fn test_parse_request_path_empty() {
        assert_eq!(parse_request_path(""), "/");
    }

    #[test]
    fn test_route_health() {
        let readiness = StaticReadiness::new(false);
        let (status, body) = route_request("GET /health HTTP/1.1\r\n\r\n", &readiness);
        assert_eq!(status, "200 OK");
        assert!(body.contains(r#""status":"ok""#));
    }

    #[test]
    fn test_route_ready_when_ready() {
        let readiness = StaticReadiness::new(true);
        let (status, body) = route_request("GET /ready HTTP/1.1\r\n\r\n", &readiness);
        assert_eq!(status, "200 OK");
        assert!(body.contains(r#""ready":true"#));
    }

    #[test]
    fn test_route_ready_when_not_ready() {
        let readiness = StaticReadiness::new(false);
        let (status, body) = route_request("GET /ready HTTP/1.1\r\n\r\n", &readiness);
        assert_eq!(status, "503 Service Unavailable");
        assert!(body.contains(r#""ready":false"#));
    }

    #[test]
    fn test_route_not_found() {
        let readiness = StaticReadiness::new(true);
        let (status, body) = route_request("GET /metrics HTTP/1.1\r\n\r\n", &readiness);
        assert_eq!(status, "404 Not Found");
        assert!(body.contains("not found"));
    }

    #[tokio::test]
    async fn test_health_server_bind_and_respond() {
        let readiness = Arc::new(StaticReadiness::new(true));
        let server = HealthServer::bind("127.0.0.1:0", readiness)
            .await
            .expect("bind should succeed");
        let addr = server.local_addr().expect("should have addr");

        // Spawn the server in background
        tokio::spawn(async move { server.run().await });

        // Make a request
        let client = reqwest::Client::new();
        let resp = client
            .get(format!("http://{}/health", addr))
            .send()
            .await
            .expect("request should succeed");
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["status"], "ok");
    }

    #[tokio::test]
    async fn test_ready_endpoint_with_providers() {
        let readiness = Arc::new(StaticReadiness::new(true));
        let server = HealthServer::bind("127.0.0.1:0", readiness)
            .await
            .expect("bind");
        let addr = server.local_addr().unwrap();
        tokio::spawn(async move { server.run().await });

        let resp = reqwest::Client::new()
            .get(format!("http://{}/ready", addr))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["ready"], true);
    }

    #[tokio::test]
    async fn test_ready_endpoint_without_providers() {
        let readiness = Arc::new(StaticReadiness::new(false));
        let server = HealthServer::bind("127.0.0.1:0", readiness)
            .await
            .expect("bind");
        let addr = server.local_addr().unwrap();
        tokio::spawn(async move { server.run().await });

        let resp = reqwest::Client::new()
            .get(format!("http://{}/ready", addr))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 503);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["ready"], false);
    }

    #[test]
    fn test_static_readiness_toggle() {
        let r = StaticReadiness::new(false);
        assert!(!r.is_ready());
        r.set_ready(true);
        assert!(r.is_ready());
    }
}
