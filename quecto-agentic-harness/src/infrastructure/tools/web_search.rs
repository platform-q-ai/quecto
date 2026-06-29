// Web search tool: Brave API with DuckDuckGo fallback.

use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;

use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};

/// Web search tool that queries the Brave Search API.
/// Falls back to DuckDuckGo HTML API if no Brave key is configured.
#[derive(Debug)]
pub struct WebSearchTool {
    api_key: Option<String>,
    client: reqwest::Client,
    brave_base: Cow<'static, str>,
    ddg_base: Cow<'static, str>,
}

impl WebSearchTool {
    pub fn new(api_key: Option<String>) -> Self {
        Self::with_client(api_key, reqwest::Client::new())
    }

    /// Create with a shared `reqwest::Client` (avoids duplicate connection pools).
    pub fn with_client(api_key: Option<String>, client: reqwest::Client) -> Self {
        Self {
            api_key,
            client,
            brave_base: Cow::Borrowed("https://api.search.brave.com"),
            ddg_base: Cow::Borrowed("https://api.duckduckgo.com"),
        }
    }

    /// Create a tool with custom base URLs (for testing with wiremock).
    pub fn with_base_urls(api_key: Option<String>, brave_base: &str, ddg_base: &str) -> Self {
        Self {
            api_key,
            client: reqwest::Client::new(),
            brave_base: Cow::Owned(brave_base.to_string()),
            ddg_base: Cow::Owned(ddg_base.to_string()),
        }
    }

    async fn search_brave(&self, query: &str, api_key: &str) -> Result<String, DomainError> {
        let url = format!(
            "{}/res/v1/web/search?q={}",
            self.brave_base,
            encode_query_param(query)
        );
        let resp = self
            .client
            .get(&url)
            .header("X-Subscription-Token", api_key)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| DomainError::Tool(format!("Brave search request failed: {}", e)))?;

        if !resp.status().is_success() {
            return Err(DomainError::Tool(format!(
                "Brave search returned status {}",
                resp.status()
            )));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| DomainError::Tool(format!("Failed to parse Brave response: {}", e)))?;

        // Extract web results
        let results = body
            .get("web")
            .and_then(|w| w.get("results"))
            .and_then(|r| r.as_array());

        match results {
            Some(items) => {
                let mut output = String::new();
                for (i, item) in items.iter().take(5).enumerate() {
                    let title = item.get("title").and_then(|v| v.as_str()).unwrap_or("?");
                    let url = item.get("url").and_then(|v| v.as_str()).unwrap_or("?");
                    let desc = item
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    output.push_str(&format!("{}. {} - {}\n   {}\n", i + 1, title, url, desc));
                }
                Ok(output)
            }
            None => Ok("No results found.".to_string()),
        }
    }

    async fn search_ddg(&self, query: &str) -> Result<String, DomainError> {
        // DuckDuckGo Instant Answer API (free, no key required)
        let url = format!(
            "{}/?q={}&format=json&no_html=1",
            self.ddg_base,
            encode_query_param(query)
        );
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| DomainError::Tool(format!("DuckDuckGo request failed: {}", e)))?;

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| DomainError::Tool(format!("Failed to parse DDG response: {}", e)))?;

        let abstract_text = body
            .get("AbstractText")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let abstract_url = body
            .get("AbstractURL")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if !abstract_text.is_empty() {
            Ok(format!("{}\nSource: {}", abstract_text, abstract_url))
        } else {
            // Try related topics
            let topics = body.get("RelatedTopics").and_then(|v| v.as_array());
            match topics {
                Some(items) if !items.is_empty() => {
                    let mut output = String::new();
                    for (i, item) in items.iter().take(5).enumerate() {
                        if let Some(text) = item.get("Text").and_then(|v| v.as_str()) {
                            let url = item.get("FirstURL").and_then(|v| v.as_str()).unwrap_or("");
                            output.push_str(&format!("{}. {} ({})\n", i + 1, text, url));
                        }
                    }
                    Ok(output)
                }
                _ => Ok("No results found.".to_string()),
            }
        }
    }
}

impl Tool for WebSearchTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "web_search".into(),
            description: "Search the web for information using Brave Search or DuckDuckGo"
                .into(),
            parameters_schema: r#"{"type":"object","properties":{"query":{"type":"string","description":"The search query"}},"required":["query"]}"#.into(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let args = arguments.to_string();
        Box::pin(async move {
            let parsed: serde_json::Value = serde_json::from_str(&args)
                .map_err(|e| DomainError::Tool(format!("invalid JSON: {}", e)))?;

            let query = parsed
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or_else(|| DomainError::Tool("missing required field: query".to_string()))?;

            let result = if let Some(ref key) = self.api_key {
                self.search_brave(query, key).await
            } else {
                self.search_ddg(query).await
            };

            match result {
                Ok(content) => Ok(ToolResult {
                    content,
                    is_error: false,
                    image_blocks: vec![],
                }),
                Err(e) => Ok(ToolResult {
                    content: format!("Search failed: {}", e),
                    is_error: true,
                    image_blocks: vec![],
                }),
            }
        })
    }
}

/// Encode a query parameter for use in search engine URLs.
///
/// Uses `+` for spaces (HTML form encoding / `application/x-www-form-urlencoded`)
/// rather than `%20` (RFC 3986). Search engine query parameters conventionally
/// use `+` encoding. This differs from the `urlencoding` crate in `Cargo.toml`
/// which uses `%20`.
fn encode_query_param(input: &str) -> String {
    use std::fmt::Write;
    let mut output = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                output.push(byte as char);
            }
            b' ' => output.push('+'),
            _ => {
                let _ = write!(output, "%{:02X}", byte);
            }
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_definition() {
        let tool = WebSearchTool::new(None);
        let def = tool.definition();
        assert_eq!(def.name, "web_search");
        assert!(def.description.contains("Search"));
    }

    #[test]
    fn test_encode_query_param() {
        assert_eq!(encode_query_param("hello world"), "hello+world");
        assert_eq!(encode_query_param("rust lang"), "rust+lang");
        assert_eq!(encode_query_param("a&b=c"), "a%26b%3Dc");
    }

    #[tokio::test]
    async fn test_missing_query() {
        let tool = WebSearchTool::new(None);
        let result = tool.execute(r#"{"wrong":"field"}"#).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_invalid_json() {
        let tool = WebSearchTool::new(None);
        let result = tool.execute("not json").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_brave_search_success() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let response = serde_json::json!({
            "web": {
                "results": [
                    {
                        "title": "Rust Programming",
                        "url": "https://rust-lang.org",
                        "description": "A systems language"
                    },
                    {
                        "title": "Rust Book",
                        "url": "https://doc.rust-lang.org/book/",
                        "description": "The Rust Programming Language"
                    }
                ]
            }
        });

        Mock::given(method("GET"))
            .and(path("/res/v1/web/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response))
            .mount(&server)
            .await;

        let tool = WebSearchTool::with_base_urls(
            Some("test-key".to_string()),
            &server.uri(),
            "http://unused",
        );
        let result = tool.execute(r#"{"query":"rust"}"#).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Rust Programming"));
        assert!(result.content.contains("rust-lang.org"));
        assert!(result.content.contains("Rust Book"));
    }

    #[tokio::test]
    async fn test_brave_search_no_results() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let response = serde_json::json!({"web": {"results": []}});

        Mock::given(method("GET"))
            .and(path("/res/v1/web/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response))
            .mount(&server)
            .await;

        let tool = WebSearchTool::with_base_urls(
            Some("test-key".to_string()),
            &server.uri(),
            "http://unused",
        );
        // Empty results array should produce empty string (no items to iterate)
        let result = tool.execute(r#"{"query":"nothing"}"#).await.unwrap();
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn test_brave_search_server_error() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/res/v1/web/search"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let tool = WebSearchTool::with_base_urls(
            Some("test-key".to_string()),
            &server.uri(),
            "http://unused",
        );
        let result = tool.execute(r#"{"query":"fail"}"#).await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Search failed"));
    }

    #[tokio::test]
    async fn test_ddg_search_abstract() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let response = serde_json::json!({
            "AbstractText": "Rust is a systems programming language.",
            "AbstractURL": "https://en.wikipedia.org/wiki/Rust",
            "RelatedTopics": []
        });

        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response))
            .mount(&server)
            .await;

        let tool = WebSearchTool::with_base_urls(None, "http://unused", &server.uri());
        let result = tool.execute(r#"{"query":"rust"}"#).await.unwrap();
        assert!(!result.is_error);
        assert!(
            result
                .content
                .contains("Rust is a systems programming language")
        );
        assert!(result.content.contains("wikipedia.org"));
    }

    #[tokio::test]
    async fn test_ddg_search_related_topics() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let response = serde_json::json!({
            "AbstractText": "",
            "AbstractURL": "",
            "RelatedTopics": [
                {"Text": "Rust (programming language)", "FirstURL": "https://example.com/rust"},
                {"Text": "Iron oxide", "FirstURL": "https://example.com/iron"}
            ]
        });

        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response))
            .mount(&server)
            .await;

        let tool = WebSearchTool::with_base_urls(None, "http://unused", &server.uri());
        let result = tool.execute(r#"{"query":"rust"}"#).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("Rust (programming language)"));
        assert!(result.content.contains("Iron oxide"));
    }

    #[tokio::test]
    async fn test_ddg_search_no_results() {
        use wiremock::matchers::method;
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let response = serde_json::json!({
            "AbstractText": "",
            "RelatedTopics": []
        });

        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&response))
            .mount(&server)
            .await;

        let tool = WebSearchTool::with_base_urls(None, "http://unused", &server.uri());
        let result = tool.execute(r#"{"query":"xyzzy"}"#).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("No results found"));
    }

    // --- #209: Shared reqwest::Client ---

    #[test]
    fn test_web_search_accepts_shared_client() {
        let client = reqwest::Client::new();
        let tool = WebSearchTool::with_client(None, client);
        assert_eq!(tool.definition().name, "web_search");
    }
}
