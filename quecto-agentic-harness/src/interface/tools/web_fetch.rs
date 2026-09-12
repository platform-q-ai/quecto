//! JSON delivery adapter for the typed web-fetch use case.
use std::borrow::Cow;
use std::future::Future;
use std::pin::Pin;

use crate::application::agent_turn::use_cases::web_fetch::{
    WebFetchError, WebFetchRequest, WebFetchUseCase,
};
use crate::domain::error::DomainError;
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};

pub struct WebFetchTool {
    use_case: WebFetchUseCase,
}

impl WebFetchTool {
    pub fn new(use_case: WebFetchUseCase) -> Self {
        Self { use_case }
    }
}

impl Tool for WebFetchTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "web_fetch".into(),
            description: "Fetch a URL and return its content as readable text. \
                          Strips HTML tags by default to save tokens. \
                          Use raw mode for JSON APIs or markdown files."
                .into(),
            parameters_schema: Cow::Borrowed(
                r#"{"type":"object","properties":{"url":{"type":"string","description":"URL to fetch (http or https)"},"raw":{"type":"boolean","description":"Return raw body without HTML stripping (default: false)"}},"required":["url"]}"#,
            ),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let request = decode_request(arguments);
        Box::pin(async move {
            let request = request?;
            self.use_case
                .execute(request)
                .await
                .map(|result| ToolResult {
                    content: result.content,
                    is_error: result.is_error,
                    image_blocks: vec![],
                    delivery_metadata: None,
                })
                .map_err(map_error)
        })
    }
}

fn decode_request(arguments: &str) -> Result<WebFetchRequest, DomainError> {
    let parsed: serde_json::Value = serde_json::from_str(arguments)
        .map_err(|error| DomainError::Tool(format!("invalid JSON: {error}")))?;
    let url = parsed
        .get("url")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| DomainError::Tool("missing required field: url".into()))?;
    Ok(WebFetchRequest {
        url: url.to_owned(),
        raw: parsed
            .get("raw")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
    })
}

fn map_error(error: WebFetchError) -> DomainError {
    match error {
        WebFetchError::InvalidUrl(message) => DomainError::Tool(format!("Invalid URL: {message}")),
        WebFetchError::RedirectLimitExceeded => {
            DomainError::Tool("Fetch failed: redirect limit exceeded".into())
        }
        WebFetchError::TimedOut => DomainError::Tool("Request timed out after 10s".into()),
        WebFetchError::ResponseTooLarge {
            actual_bytes,
            max_bytes,
        } => DomainError::Tool(match actual_bytes {
            Some(actual) => format!("Response too large: {actual} bytes (max {max_bytes})"),
            None => format!("Response too large: >{max_bytes} bytes (max {max_bytes})"),
        }),
        WebFetchError::Resolution(message) => {
            DomainError::Tool(format!("Fetch failed: DNS resolution failed: {message}"))
        }
        WebFetchError::Connection(message) | WebFetchError::Transport(message) => {
            DomainError::Tool(format!("Fetch failed: {message}"))
        }
        WebFetchError::Read(message) => {
            DomainError::Tool(format!("Failed to read response body: {message}"))
        }
    }
}

#[cfg(test)]
#[path = "web_fetch_tests.rs"]
mod tests;
