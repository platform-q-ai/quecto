use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::Value;

mod config;
mod model;
pub use config::Config;
pub use model::{McpTool, QuectoToolRegistration, RegisteredMcpTools};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

#[derive(Debug, Error)]
pub enum QuectoMcpError {
    #[error("invalid MCP tool name: {0}")]
    InvalidToolName(String),
    #[error("tool name collision: {safe_name} maps to both {first_mcp_name} and {second_mcp_name}")]
    ToolNameCollision {
        safe_name: String,
        first_mcp_name: String,
        second_mcp_name: String,
    },
    #[error("invalid token source: {0}")]
    InvalidTokenSource(String),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("MCP error: {0}")]
    Mcp(String),
    #[error("unsupported option: {0}")]
    UnsupportedOption(String),
    #[error("response too large: {0} bytes exceeds {1} byte limit")]
    ResponseTooLarge(u64, usize),
    #[error("Quecto UDS error: {0}")]
    Quecto(String),
}

pub type Result<T> = std::result::Result<T, QuectoMcpError>;

const MAX_MCP_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_UDS_LINE_BYTES: usize = 1024 * 1024;
const MAX_CONCURRENT_TOOL_CALLS: usize = 8;

pub fn mcp_name_to_quecto_name(name: &str) -> Result<String> {
    if name.trim().is_empty() {
        return Err(QuectoMcpError::InvalidToolName(name.to_string()));
    }

    let mut out = String::with_capacity(name.len());
    let mut prev_underscore = false;

    for ch in name.chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            ch.to_ascii_lowercase()
        } else {
            '_'
        };
        if mapped == '_' {
            if !prev_underscore {
                out.push('_');
                prev_underscore = true;
            }
        } else {
            out.push(mapped);
            prev_underscore = false;
        }
    }

    let out = out.trim_matches('_').to_string();
    if out.is_empty() || !out.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
        return Err(QuectoMcpError::InvalidToolName(name.to_string()));
    }
    Ok(out)
}

pub fn filter_tools(
    tools: &[McpTool],
    prefixes: &[String],
    allowlist: &[String],
    denylist: &[String],
) -> Result<Vec<McpTool>> {
    let allow: HashSet<&str> = allowlist.iter().map(String::as_str).collect();
    let deny: HashSet<&str> = denylist.iter().map(String::as_str).collect();

    Ok(tools
        .iter()
        .filter(|tool| {
            (prefixes.is_empty() || prefixes.iter().any(|prefix| tool.name.starts_with(prefix)))
                && (allow.is_empty() || allow.contains(tool.name.as_str()))
                && !deny.contains(tool.name.as_str())
        })
        .cloned()
        .collect())
}

pub fn build_mapping(tools: &[McpTool]) -> Result<HashMap<String, String>> {
    build_mapping_with_name_prefix(tools, "")
}

pub fn build_mapping_with_name_prefix(
    tools: &[McpTool],
    name_prefix: &str,
) -> Result<HashMap<String, String>> {
    let mut mapping = HashMap::new();
    let mut seen_mcp_names = HashSet::new();
    for tool in tools {
        if !seen_mcp_names.insert(tool.name.clone()) {
            return Err(QuectoMcpError::ToolNameCollision {
                safe_name: mcp_name_to_quecto_name(&tool.name)?,
                first_mcp_name: tool.name.clone(),
                second_mcp_name: tool.name.clone(),
            });
        }
        let safe = quecto_name_with_prefix(&tool.name, name_prefix)?;
        if let Some(existing) = mapping.insert(safe.clone(), tool.name.clone()) {
            return Err(QuectoMcpError::ToolNameCollision {
                safe_name: safe,
                first_mcp_name: existing,
                second_mcp_name: tool.name.clone(),
            });
        }
    }
    Ok(mapping)
}

pub fn build_registration(tool: &McpTool) -> Result<QuectoToolRegistration> {
    build_registration_with_name_prefix(tool, "")
}

fn quecto_name_with_prefix(mcp_name: &str, name_prefix: &str) -> Result<String> {
    let safe = format!("{name_prefix}{}", mcp_name_to_quecto_name(mcp_name)?);
    if safe != mcp_name_to_quecto_name(&safe)? {
        return Err(QuectoMcpError::InvalidToolName(safe));
    }
    Ok(safe)
}

pub fn build_registration_with_name_prefix(
    tool: &McpTool,
    name_prefix: &str,
) -> Result<QuectoToolRegistration> {
    Ok(QuectoToolRegistration {
        name: quecto_name_with_prefix(&tool.name, name_prefix)?,
        description: if tool.description.is_empty() {
            format!("MCP tool {}", tool.name)
        } else {
            tool.description.clone()
        },
        parameters_schema: serde_json::to_string(&tool.input_schema)?,
    })
}

pub fn build_registrations(tools: &[McpTool]) -> Result<Vec<QuectoToolRegistration>> {
    build_registrations_with_name_prefix(tools, "")
}

pub fn build_registrations_with_name_prefix(
    tools: &[McpTool],
    name_prefix: &str,
) -> Result<Vec<QuectoToolRegistration>> {
    build_mapping_with_name_prefix(tools, name_prefix)?;
    tools
        .iter()
        .map(|tool| build_registration_with_name_prefix(tool, name_prefix))
        .collect()
}

#[derive(Debug, Clone)]
pub struct McpClient {
    base_url: String,
    token: String,
    client: reqwest::Client,
    session_id: Arc<tokio::sync::Mutex<Option<String>>>,
    server_name: Arc<tokio::sync::Mutex<Option<String>>>,
}

impl McpClient {
    pub fn new(base_url: String, token: String) -> Self {
        Self::new_with_timeout(base_url, token, Duration::from_secs(30))
    }

    pub fn new_with_timeout(base_url: String, token: String, timeout: Duration) -> Self {
        Self {
            base_url,
            token,
            client: reqwest::Client::builder()
                .timeout(timeout)
                .build()
                .expect("reqwest client with timeout"),
            session_id: Arc::new(tokio::sync::Mutex::new(None)),
            server_name: Arc::new(tokio::sync::Mutex::new(None)),
        }
    }

    pub async fn initialize(&self, server_name: &str) -> Result<()> {
        let params = serde_json::json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": {"name": "quecto-mcp", "version": env!("CARGO_PKG_VERSION"), "targetServer": server_name}
        });
        let (_body, session) = self.post_json_rpc("initialize", params).await?;
        *self.server_name.lock().await = Some(server_name.to_string());
        if let Some(session) = session {
            *self.session_id.lock().await = Some(session);
        }
        self.post_json_rpc("notifications/initialized", serde_json::json!({}))
            .await?;
        Ok(())
    }

    pub async fn list_tools(&self) -> Result<Vec<McpTool>> {
        let (body, _) = self
            .post_json_rpc("tools/list", serde_json::json!({}))
            .await?;
        let tools = body
            .get("result")
            .and_then(|r| r.get("tools"))
            .and_then(Value::as_array)
            .ok_or_else(|| QuectoMcpError::Mcp(format!("invalid tools/list response: {body}")))?;

        tools
            .iter()
            .map(|tool| {
                let name = tool
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        QuectoMcpError::Mcp(format!("invalid tools/list tool entry: {tool}"))
                    })?
                    .to_string();
                let description = tool
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let input_schema = tool
                    .get("inputSchema")
                    .or_else(|| tool.get("input_schema"))
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({"type": "object"}));
                Ok(McpTool {
                    name,
                    description,
                    input_schema,
                })
            })
            .collect()
    }

    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<String> {
        let params = serde_json::json!({"name": name, "arguments": arguments});
        match self.post_json_rpc("tools/call", params.clone()).await {
            Ok((body, _)) => Ok(format_mcp_result(body.get("result").unwrap_or(&body))),
            Err(err) if is_recoverable_session_error(&err) => {
                if let Some(server_name) = self.server_name.lock().await.clone() {
                    *self.session_id.lock().await = None;
                    self.initialize(&server_name).await?;
                    let (body, _) = self.post_json_rpc("tools/call", params).await?;
                    Ok(format_mcp_result(body.get("result").unwrap_or(&body)))
                } else {
                    Err(err)
                }
            }
            Err(err) => Err(err),
        }
    }

    async fn post_json_rpc(&self, method: &str, params: Value) -> Result<(Value, Option<String>)> {
        let mut req = self
            .client
            .post(&self.base_url)
            .bearer_auth(&self.token)
            .header("accept", "application/json, text/event-stream")
            .json(&serde_json::json!({"jsonrpc": "2.0", "id": uuid::Uuid::new_v4().to_string(), "method": method, "params": params}));
        if let Some(session_id) = self.session_id.lock().await.clone() {
            req = req.header("mcp-session-id", session_id);
        }
        let resp = req.send().await?;
        let session = resp
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
            .map(ToOwned::to_owned);
        let status = resp.status();
        let text = read_limited_response_text(resp, MAX_MCP_RESPONSE_BYTES).await?;
        let body: Value = if text.trim().is_empty() {
            serde_json::json!({})
        } else {
            serde_json::from_str(&text)?
        };
        if !status.is_success() {
            return Err(QuectoMcpError::Mcp(redact(&format!(
                "HTTP {status}: {body}"
            ))));
        }
        if let Some(error) = body.get("error") {
            return Err(QuectoMcpError::Mcp(redact(&error.to_string())));
        }
        Ok((body, session))
    }
}

async fn read_limited_response_text(resp: reqwest::Response, max_bytes: usize) -> Result<String> {
    if let Some(len) = resp.content_length() {
        if len > max_bytes as u64 {
            return Err(QuectoMcpError::ResponseTooLarge(len, max_bytes));
        }
    }

    let mut body = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if body.len().saturating_add(chunk.len()) > max_bytes {
            return Err(QuectoMcpError::ResponseTooLarge(
                body.len().saturating_add(chunk.len()) as u64,
                max_bytes,
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&body).into_owned())
}

fn is_recoverable_session_error(err: &QuectoMcpError) -> bool {
    let message = err.to_string();
    message.contains("HTTP 401") || message.contains("HTTP 404") || message.contains("session")
}

fn format_mcp_result(result: &Value) -> String {
    if let Some(content) = result.get("content").and_then(Value::as_array) {
        let text = content
            .iter()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");
        if !text.is_empty() {
            return text;
        }
    }
    serde_json::to_string_pretty(result).unwrap_or_else(|_| result.to_string())
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum QuectoEvent {
    ExecuteTool {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        #[serde(default)]
        arguments: Value,
    },
    Response {
        id: Option<String>,
        command: Option<String>,
        success: bool,
        error: Option<String>,
    },
    #[serde(other)]
    Other,
}

pub async fn register_tools(socket: &Path, registrations: &[QuectoToolRegistration]) -> Result<()> {
    let mut stream = UnixStream::connect(socket).await?;
    let command = serde_json::json!({"type": "register_tools", "id": "quecto-mcp-register", "tools": registrations});
    let mut line = serde_json::to_string(&command)?;
    line.push('\n');
    stream.write_all(line.as_bytes()).await?;
    stream.flush().await?;
    Ok(())
}

pub async fn run_extension(config: Config) -> Result<()> {
    tracing::info!(
        socket = %config.socket.display(),
        mcp_url = %redact_url(&config.mcp_url),
        prefixes = ?config.tool_prefixes,
        allowlist_len = config.tool_allowlist.len(),
        denylist_len = config.tool_denylist.len(),
        has_name_prefix = !config.name_prefix.is_empty(),
        "starting quecto-mcp"
    );
    let mcp = McpClient::new_with_timeout(
        config.mcp_url.clone(),
        config.mcp_token.clone(),
        config.timeout,
    );
    mcp.initialize(&config.mcp_server_name).await?;
    let discovered = mcp.list_tools().await?;
    let filtered = filter_tools(
        &discovered,
        &config.tool_prefixes,
        &config.tool_allowlist,
        &config.tool_denylist,
    )?;
    let mapping = build_mapping_with_name_prefix(&filtered, &config.name_prefix)?;
    let registrations = build_registrations_with_name_prefix(&filtered, &config.name_prefix)?;

    tracing::info!(
        discovered = discovered.len(),
        registered = registrations.len(),
        "registering MCP tools with Quecto"
    );

    serve_uds_extension(
        &config.socket,
        RegisteredMcpTools {
            registrations,
            mapping,
        },
        mcp,
        config.register_timeout,
    )
    .await
}

pub async fn serve_uds_extension(
    socket: &Path,
    tools: RegisteredMcpTools,
    mcp: McpClient,
    register_timeout: Duration,
) -> Result<()> {
    let stream = tokio::time::timeout(register_timeout, UnixStream::connect(socket))
        .await
        .map_err(|_| {
            QuectoMcpError::Quecto("timed out connecting to Quecto UDS socket".into())
        })??;
    let (read_half, write_half) = stream.into_split();
    let (result_tx, mut result_rx) = tokio::sync::mpsc::channel::<Value>(64);
    let writer_task = tokio::spawn(async move {
        let mut write_half = write_half;
        while let Some(value) = result_rx.recv().await {
            if let Err(err) = write_json_line(&mut write_half, &value).await {
                tracing::warn!(error = %err, "failed to write Quecto tool_result");
                break;
            }
        }
    });

    let register = serde_json::json!({"type": "register_tools", "id": "quecto-mcp-register", "tools": tools.registrations});
    result_tx
        .send(register)
        .await
        .map_err(|_| QuectoMcpError::Quecto("UDS writer task stopped".into()))?;

    let mut reader = BufReader::new(read_half);
    await_register_response(&mut reader, register_timeout).await?;
    let semaphore = Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_TOOL_CALLS));
    loop {
        let Some(line) = read_limited_line(&mut reader, MAX_UDS_LINE_BYTES).await? else {
            drop(result_tx);
            let _ = writer_task.await;
            return Ok(());
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let event: QuectoEvent = match serde_json::from_str(trimmed) {
            Ok(event) => event,
            Err(err) => {
                tracing::warn!(error = %err, "ignoring invalid Quecto UDS event");
                continue;
            }
        };
        if let QuectoEvent::ExecuteTool {
            tool_call_id,
            tool_name,
            arguments,
        } = event
        {
            let mcp_name = tools.mapping.get(&tool_name).cloned();
            let mcp = mcp.clone();
            let result_tx = result_tx.clone();
            let semaphore = semaphore.clone();
            tokio::spawn(async move {
                let Ok(_permit) = semaphore.acquire_owned().await else {
                    return;
                };
                let (content, is_error) = match mcp_name {
                    Some(mcp_name) => match arguments_to_json(arguments) {
                        Ok(args) => {
                            let start = std::time::Instant::now();
                            tracing::info!(quecto_tool = %tool_name, mcp_tool = %mcp_name, "executing MCP-backed tool");
                            let result = match mcp.call_tool(&mcp_name, args).await {
                                Ok(content) => (content, false),
                                Err(err) => (redact(&err.to_string()), true),
                            };
                            tracing::info!(
                                quecto_tool = %tool_name,
                                mcp_tool = %mcp_name,
                                elapsed_ms = start.elapsed().as_millis() as u64,
                                is_error = result.1,
                                "finished MCP-backed tool"
                            );
                            result
                        }
                        Err(err) => (err, true),
                    },
                    None => (format!("Unknown MCP-backed tool: {tool_name}"), true),
                };
                let result = serde_json::json!({"type": "tool_result", "toolCallId": tool_call_id, "content": content, "isError": is_error});
                let _ = result_tx.send(result).await;
            });
        }
    }
}

fn arguments_to_json(arguments: Value) -> std::result::Result<Value, String> {
    match arguments {
        Value::String(raw) => {
            serde_json::from_str(&raw).map_err(|err| format!("Invalid JSON arguments: {err}"))
        }
        other => Err(format!(
            "Invalid execute_tool arguments: expected JSON string, got {}",
            json_type_name(&other)
        )),
    }
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

async fn await_register_response(
    reader: &mut BufReader<tokio::net::unix::OwnedReadHalf>,
    register_timeout: Duration,
) -> Result<()> {
    loop {
        let Some(line) = tokio::time::timeout(
            register_timeout,
            read_limited_line(reader, MAX_UDS_LINE_BYTES),
        )
        .await
        .map_err(|_| {
            QuectoMcpError::Quecto("timed out waiting for register_tools response".into())
        })??
        else {
            return Err(QuectoMcpError::Quecto(
                "Quecto UDS disconnected before register_tools response".into(),
            ));
        };
        let event: QuectoEvent = serde_json::from_str(line.trim())?;
        if let QuectoEvent::Response {
            id,
            command,
            success,
            error,
        } = event
        {
            if id.as_deref() == Some("quecto-mcp-register")
                || command.as_deref() == Some("register_tools")
            {
                return if success {
                    Ok(())
                } else {
                    Err(QuectoMcpError::Quecto(error.unwrap_or_else(|| {
                        "register_tools failed without an error message".to_string()
                    })))
                };
            }
        }
    }
}

async fn read_limited_line<R>(reader: &mut R, max_bytes: usize) -> Result<Option<String>>
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    let mut buf = Vec::new();
    loop {
        let before = buf.len();
        let bytes = reader.read_until(b'\n', &mut buf).await?;
        if bytes == 0 {
            if buf.is_empty() {
                return Ok(None);
            }
            break;
        }
        if buf.len() > max_bytes {
            return Err(QuectoMcpError::Quecto(format!(
                "UDS line exceeds {max_bytes} byte limit"
            )));
        }
        if buf[before..].contains(&b'\n') {
            break;
        }
    }
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

async fn write_json_line(
    writer: &mut tokio::net::unix::OwnedWriteHalf,
    value: &Value,
) -> Result<()> {
    let mut line = serde_json::to_string(value)?;
    line.push('\n');
    writer.write_all(line.as_bytes()).await?;
    writer.flush().await?;
    Ok(())
}

mod redact;
pub use redact::redact;
use redact::redact_url;

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
