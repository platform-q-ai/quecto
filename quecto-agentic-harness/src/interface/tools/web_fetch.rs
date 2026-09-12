use crate::{
    application::agent_turn::use_cases::web_fetch::{
        FetchFailure, WebFetchError, WebFetchResult, WebFetchUseCase,
    },
    domain::{
        error::DomainError,
        tool::{Tool, ToolDefinition, ToolResult},
    },
};
use std::{borrow::Cow, future::Future, pin::Pin, sync::Arc};
pub struct WebFetchTool {
    use_case: Arc<WebFetchUseCase>,
}
impl WebFetchTool {
    pub fn new(use_case: Arc<WebFetchUseCase>) -> Self {
        Self { use_case }
    }
}
fn result(content: String, is_error: bool) -> ToolResult {
    ToolResult {
        content,
        is_error,
        image_blocks: vec![],
        delivery_metadata: None,
    }
}
impl Tool for WebFetchTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition{name:"web_fetch".into(),description:"Fetch a URL and return its content as readable text. Strips HTML tags by default to save tokens. Use raw mode for JSON APIs or markdown files.".into(),parameters_schema:Cow::Borrowed(r#"{"type":"object","properties":{"url":{"type":"string","description":"URL to fetch (http or https)"},"raw":{"type":"boolean","description":"Return raw body without HTML stripping (default: false)"}},"required":["url"]}"#)}
    }
    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let args = arguments.to_owned();
        Box::pin(async move {
            let parsed: serde_json::Value = serde_json::from_str(&args)
                .map_err(|e| DomainError::Tool(format!("invalid JSON: {e}")))?;
            let url = parsed
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or_else(|| DomainError::Tool("missing required field: url".into()))?;
            let raw = parsed.get("raw").and_then(|v| v.as_bool()).unwrap_or(false);
            match self.use_case.execute(url, raw).await {
                Ok(WebFetchResult::Success(s)) => Ok(result(s, false)),
                Ok(WebFetchResult::UnsupportedScheme) => Ok(result(
                    format!(
                        "Invalid URL scheme: only http:// and https:// are allowed. Got: {url}"
                    ),
                    true,
                )),
                Ok(WebFetchResult::RestrictedInitialHost(h)) => Ok(result(
                    format!("Blocked: URL points to a restricted address ({h})"),
                    true,
                )),
                Ok(WebFetchResult::NonSuccessStatus(s)) => {
                    Ok(result(format!("HTTP {} fetching {url}", s.0), true))
                }
                Err(WebFetchError::InvalidUrl(e)) => {
                    Err(DomainError::Tool(format!("Invalid URL: {e}")))
                }
                Err(WebFetchError::Fetch(f)) => Err(map_failure(f, url)),
            }
        })
    }
}
fn map_failure(f: FetchFailure, url: &str) -> DomainError {
    DomainError::Tool(match f {
        FetchFailure::TimedOut => format!("Request timed out after 10s: {url}"),
        FetchFailure::TooLarge {
            actual_bytes: Some(n),
            max_bytes: m,
        } => format!("Response too large: {n} bytes (max {m})"),
        FetchFailure::TooLarge {
            actual_bytes: None,
            max_bytes: m,
        } => format!("Response too large: >{m} bytes (max {m})"),
        FetchFailure::Read(e) => format!("Failed to read response body: {e}"),
        FetchFailure::Transport(e) => format!("Fetch failed: {e}"),
    })
}
