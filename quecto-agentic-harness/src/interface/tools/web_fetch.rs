use crate::{
    application::agent_turn::use_cases::web_fetch::{
        FetchFailure, HtmlView, WebFetchError, WebFetchResult, WebFetchUseCase,
    },
    application::tools::ports::Tool,
    domain::{
        error::DomainError,
        tool::{ToolDefinition, ToolResult},
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
    /// Reads only: calls may overlap (#2169).
    fn overlaps_safely(&self, _arguments: &str) -> bool {
        true
    }

    fn definition(&self) -> ToolDefinition {
        ToolDefinition{name:"web_fetch".into(),description:"Fetch a URL and return its content as text: HTML is made readable (tags stripped; only the main content when the page marks one, unless main_only is false) unless raw, other text (JSON, markdown, code, CSV...) comes back as served, and binary content (images, archives, PDFs) is named, not shown.".into(),parameters_schema:Cow::Borrowed(r#"{"type":"object","properties":{"url":{"type":"string","description":"URL to fetch (http or https)"},"raw":{"type":"boolean","description":"For HTML: return the markup as served instead of readable text (default: false)"},"main_only":{"type":"boolean","description":"For HTML: when the page marks its main content, return only that (default: true); false returns the whole page as readable text"}},"required":["url"]}"#)}
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
            let raw = flag(&parsed, "raw", false)?;
            let main_only = flag(&parsed, "main_only", true)?;
            let view = match (raw, main_only) {
                (true, _) => HtmlView::Markup,
                (false, true) => HtmlView::MainContent,
                (false, false) => HtmlView::WholePage,
            };
            match self.use_case.execute(url, view).await {
                Ok(WebFetchResult::Success(s)) => Ok(result(s, false)),
                Ok(WebFetchResult::Binary {
                    content_type,
                    bytes,
                }) => Ok(result(
                    format!(
                        "{url} is binary content ({}, {bytes} bytes), not shown: web_fetch returns text and HTML only",
                        content_type.as_deref().unwrap_or("no content type")
                    ),
                    false,
                )),
                Ok(WebFetchResult::UnsupportedScheme) => Ok(result(
                    format!(
                        "Invalid URL scheme: only http:// and https:// are allowed. Got: {url}"
                    ),
                    true,
                )),
                Ok(WebFetchResult::RestrictedInitialHost(h)) => Ok(result(
                    format!("Blocked: URL points to a restricted address; refused: {h}"),
                    true,
                )),
                Ok(WebFetchResult::NonSuccessStatus(status)) => {
                    let displayed = status.reason.as_ref().map_or_else(
                        || status.code.to_string(),
                        |reason| format!("{} {reason}", status.code),
                    );
                    Ok(result(format!("HTTP {displayed} fetching {url}"), true))
                }
                Err(WebFetchError::InvalidUrl(e)) => {
                    Err(DomainError::Tool(format!("Invalid URL: {e}")))
                }
                Err(WebFetchError::Fetch(f)) => Err(map_failure(f, url)),
            }
        })
    }
}
/// A boolean argument: absent or null is its default; anything else but a
/// boolean is refused, naming the argument (#2165 review).
fn flag(args: &serde_json::Value, name: &str, default: bool) -> Result<bool, DomainError> {
    match args.get(name) {
        Some(serde_json::Value::Bool(value)) => Ok(*value),
        None | Some(serde_json::Value::Null) => Ok(default),
        Some(other) => Err(DomainError::Tool(format!(
            "invalid argument {name}: expected a boolean (true or false), got {other}"
        ))),
    }
}
#[cfg(test)]
#[path = "web_fetch_tests.rs"]
mod tests;

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
        FetchFailure::Refused(reason) => {
            format!("Blocked: {url} reaches a restricted address; refused: {reason}")
        }
        FetchFailure::BadRedirect(reason) => format!("Redirect refused: {reason}"),
    })
}
