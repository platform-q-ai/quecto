use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
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
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("MCP error: {0}")]
    Mcp(String),
    #[error("Quecto UDS error: {0}")]
    Quecto(String),
}

pub type Result<T> = std::result::Result<T, QuectoMcpError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuectoToolRegistration {
    pub name: String,
    pub description: String,
    #[serde(rename = "parametersSchema")]
    pub parameters_schema: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolMapping {
    pub quecto_name: String,
    pub mcp_name: String,
}

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
    let mut mapping = HashMap::new();
    for tool in tools {
        let safe = mcp_name_to_quecto_name(&tool.name)?;
        if let Some(existing) = mapping.insert(safe.clone(), tool.name.clone()) {
            if existing != tool.name {
                return Err(QuectoMcpError::ToolNameCollision {
                    safe_name: safe,
                    first_mcp_name: existing,
                    second_mcp_name: tool.name.clone(),
                });
            }
        }
    }
    Ok(mapping)
}

pub fn build_registration(tool: &McpTool) -> Result<QuectoToolRegistration> {
    Ok(QuectoToolRegistration {
        name: mcp_name_to_quecto_name(&tool.name)?,
        description: if tool.description.is_empty() {
            format!("MCP tool {}", tool.name)
        } else {
            tool.description.clone()
        },
        parameters_schema: serde_json::to_string(&tool.input_schema)?,
    })
}

pub fn build_registrations(tools: &[McpTool]) -> Result<Vec<QuectoToolRegistration>> {
    tools.iter().map(build_registration).collect()
}

#[derive(Debug, Clone)]
pub struct Config {
    pub socket: PathBuf,
    pub mcp_url: String,
    pub mcp_token: String,
    pub mcp_server_name: String,
    pub tool_prefixes: Vec<String>,
    pub tool_allowlist: Vec<String>,
    pub tool_denylist: Vec<String>,
    pub timeout: Duration,
}

impl Config {
    pub fn from_env_and_args(args: impl IntoIterator<Item = String>) -> Result<Self> {
        let mut socket = std::env::var("QUECTO_SOCKET").ok().map(PathBuf::from);
        let mut mcp_url = std::env::var("PERME8_MCP_URL").ok();
        let mut mcp_token = std::env::var("PERME8_MCP_TOKEN").ok();
        let mut tool_prefixes = std::env::var("QUECTO_MCP_TOOL_PREFIXES")
            .ok()
            .map(|s| split_csv(&s))
            .unwrap_or_default();
        let mut allowlist = Vec::new();
        let mut denylist = Vec::new();
        let mut server_name = "perme8-mcp".to_string();
        let mut timeout = Duration::from_secs(30);

        let mut iter = args.into_iter().skip(1);
        while let Some(arg) = iter.next() {
            match arg.as_str() {
                "--socket" => socket = iter.next().map(PathBuf::from),
                "--mcp-url" => mcp_url = iter.next(),
                "--mcp-token" => mcp_token = iter.next(),
                "--mcp-server-name" => server_name = iter.next().unwrap_or(server_name),
                "--tool-prefix" => {
                    if let Some(prefix) = iter.next() {
                        tool_prefixes.push(prefix);
                    }
                }
                "--tool-allowlist" => {
                    if let Some(value) = iter.next() {
                        allowlist.extend(split_csv(&value));
                    }
                }
                "--tool-denylist" => {
                    if let Some(value) = iter.next() {
                        denylist.extend(split_csv(&value));
                    }
                }
                "--timeout" => {
                    if let Some(value) = iter.next().and_then(|s| s.parse::<u64>().ok()) {
                        timeout = Duration::from_secs(value);
                    }
                }
                _ => {}
            }
        }

        Ok(Self {
            socket: socket
                .ok_or_else(|| QuectoMcpError::Quecto("missing --socket / QUECTO_SOCKET".into()))?,
            mcp_url: mcp_url
                .ok_or_else(|| QuectoMcpError::Mcp("missing --mcp-url / PERME8_MCP_URL".into()))?,
            mcp_token: mcp_token.ok_or_else(|| {
                QuectoMcpError::Mcp("missing --mcp-token / PERME8_MCP_TOKEN".into())
            })?,
            mcp_server_name: server_name,
            tool_prefixes,
            tool_allowlist: allowlist,
            tool_denylist: denylist,
            timeout,
        })
    }
}

fn split_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

#[derive(Debug, Clone)]
pub struct McpClient {
    base_url: String,
    token: String,
    client: reqwest::Client,
    session_id: Option<String>,
}

impl McpClient {
    pub fn new(base_url: String, token: String) -> Self {
        Self {
            base_url,
            token,
            client: reqwest::Client::new(),
            session_id: None,
        }
    }

    pub async fn initialize(&mut self, server_name: &str) -> Result<()> {
        let params = serde_json::json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": {"name": "quecto-mcp", "version": env!("CARGO_PKG_VERSION"), "targetServer": server_name}
        });
        let (_body, session) = self.post_json_rpc("initialize", params).await?;
        if let Some(session) = session {
            self.session_id = Some(session);
        }
        let _ = self
            .post_json_rpc("notifications/initialized", serde_json::json!({}))
            .await;
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

        Ok(tools
            .iter()
            .filter_map(|tool| {
                let name = tool.get("name")?.as_str()?.to_string();
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
                Some(McpTool {
                    name,
                    description,
                    input_schema,
                })
            })
            .collect())
    }

    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<String> {
        let params = serde_json::json!({"name": name, "arguments": arguments});
        let (body, _) = self.post_json_rpc("tools/call", params).await?;
        if let Some(error) = body.get("error") {
            return Err(QuectoMcpError::Mcp(error.to_string()));
        }
        Ok(format_mcp_result(body.get("result").unwrap_or(&body)))
    }

    async fn post_json_rpc(&self, method: &str, params: Value) -> Result<(Value, Option<String>)> {
        let mut req = self
            .client
            .post(&self.base_url)
            .bearer_auth(&self.token)
            .json(&serde_json::json!({"jsonrpc": "2.0", "id": uuid::Uuid::new_v4().to_string(), "method": method, "params": params}));
        if let Some(session_id) = &self.session_id {
            req = req.header("mcp-session-id", session_id);
        }
        let resp = req.send().await?;
        let session = resp
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
            .map(ToOwned::to_owned);
        let status = resp.status();
        let body: Value = resp.json().await?;
        if !status.is_success() {
            return Err(QuectoMcpError::Mcp(format!("HTTP {status}: {body}")));
        }
        Ok((body, session))
    }
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
        arguments: String,
    },
    Response {
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
    let mut mcp = McpClient::new(config.mcp_url.clone(), config.mcp_token.clone());
    mcp.initialize(&config.mcp_server_name).await?;
    let discovered = mcp.list_tools().await?;
    let filtered = filter_tools(
        &discovered,
        &config.tool_prefixes,
        &config.tool_allowlist,
        &config.tool_denylist,
    )?;
    let mapping = build_mapping(&filtered)?;
    let registrations = build_registrations(&filtered)?;

    tracing::info!(
        discovered = discovered.len(),
        registered = registrations.len(),
        "registering MCP tools with Quecto"
    );

    serve_uds_extension(&config.socket, registrations, mapping, mcp).await
}

pub async fn serve_uds_extension(
    socket: &Path,
    registrations: Vec<QuectoToolRegistration>,
    mapping: HashMap<String, String>,
    mcp: McpClient,
) -> Result<()> {
    let stream = UnixStream::connect(socket).await?;
    let (read_half, mut write_half) = stream.into_split();
    let register = serde_json::json!({"type": "register_tools", "id": "quecto-mcp-register", "tools": registrations});
    write_json_line(&mut write_half, &register).await?;

    let mut reader = BufReader::new(read_half);
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).await? == 0 {
            return Ok(());
        }
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
            let (content, is_error) = match mapping.get(&tool_name) {
                Some(mcp_name) => match serde_json::from_str::<Value>(&arguments) {
                    Ok(args) => match mcp.call_tool(mcp_name, args).await {
                        Ok(content) => (content, false),
                        Err(err) => (redact(&err.to_string()), true),
                    },
                    Err(err) => (format!("Invalid JSON arguments: {err}"), true),
                },
                None => (format!("Unknown MCP-backed tool: {tool_name}"), true),
            };
            let result = serde_json::json!({"type": "tool_result", "toolCallId": tool_call_id, "content": content, "isError": is_error});
            write_json_line(&mut write_half, &result).await?;
        }
    }
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

pub fn redact(input: &str) -> String {
    input
        .replace("Authorization", "<redacted-header>")
        .replace("Bearer ", "Bearer <redacted>")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::UnixListener;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn maps_dotted_names_to_safe_names() {
        assert_eq!(
            mcp_name_to_quecto_name("community.channels.send_message").unwrap(),
            "community_channels_send_message"
        );
    }

    #[test]
    fn detects_name_collisions() {
        let tools = vec![
            McpTool {
                name: "a.b".into(),
                description: "".into(),
                input_schema: serde_json::json!({}),
            },
            McpTool {
                name: "a_b".into(),
                description: "".into(),
                input_schema: serde_json::json!({}),
            },
        ];
        assert!(matches!(
            build_mapping(&tools),
            Err(QuectoMcpError::ToolNameCollision { .. })
        ));
    }

    #[test]
    fn filters_tools() {
        let tools = vec![
            McpTool {
                name: "community.feed.list".into(),
                description: "".into(),
                input_schema: serde_json::json!({}),
            },
            McpTool {
                name: "ticket.read".into(),
                description: "".into(),
                input_schema: serde_json::json!({}),
            },
        ];
        let filtered = filter_tools(&tools, &["community.".into()], &[], &[]).unwrap();
        assert_eq!(filtered.len(), 1);
    }

    #[tokio::test]
    async fn mcp_client_lists_tools_and_calls_tool() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/"))
            .and(body_partial_json(
                serde_json::json!({"method": "initialize"}),
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("mcp-session-id", "session-1")
                    .set_body_json(
                        serde_json::json!({"jsonrpc": "2.0", "id": "init", "result": {}}),
                    ),
            )
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/"))
            .and(body_partial_json(
                serde_json::json!({"method": "notifications/initialized"}),
            ))
            .respond_with(ResponseTemplate::new(202).set_body_json(serde_json::json!({})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/"))
            .and(body_partial_json(
                serde_json::json!({"method": "tools/list"}),
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": "list",
                "result": {"tools": [{
                    "name": "community.feed.list",
                    "description": "List feed posts",
                    "inputSchema": {"type": "object"}
                }]}
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/"))
            .and(body_partial_json(serde_json::json!({
                "method": "tools/call",
                "params": {"name": "community.feed.list", "arguments": {"limit": 1}}
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": "call",
                "result": {"content": [{"type": "text", "text": "feed item"}]}
            })))
            .mount(&server)
            .await;

        let mut client = McpClient::new(server.uri(), "secret-token".into());
        client.initialize("perme8-mcp").await.unwrap();
        let tools = client.list_tools().await.unwrap();
        assert_eq!(tools[0].name, "community.feed.list");
        assert_eq!(tools[0].description, "List feed posts");

        let result = client
            .call_tool("community.feed.list", serde_json::json!({"limit": 1}))
            .await
            .unwrap();
        assert_eq!(result, "feed item");
    }

    #[tokio::test]
    async fn uds_extension_registers_tools_and_returns_tool_result() {
        let mcp_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/"))
            .and(body_partial_json(serde_json::json!({
                "method": "tools/call",
                "params": {"name": "community.chat.send_dm", "arguments": {"message": "hi"}}
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": "call",
                "result": {"content": [{"type": "text", "text": "sent"}]}
            })))
            .mount(&mcp_server)
            .await;

        let temp = tempfile::tempdir().unwrap();
        let socket = temp.path().join("agent.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let server_task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(&mut stream);
            let mut register_line = String::new();
            reader.read_line(&mut register_line).await.unwrap();
            let register_json: Value = serde_json::from_str(register_line.trim()).unwrap();
            assert_eq!(register_json["type"], "register_tools");
            assert_eq!(register_json["tools"][0]["name"], "community_chat_send_dm");
            drop(reader);

            let execute = serde_json::json!({
                "type": "execute_tool",
                "toolCallId": "uds-1",
                "toolName": "community_chat_send_dm",
                "arguments": "{\"message\":\"hi\"}"
            });
            let mut line = serde_json::to_string(&execute).unwrap();
            line.push('\n');
            stream.write_all(line.as_bytes()).await.unwrap();

            let mut reader = BufReader::new(&mut stream);
            let mut result_line = String::new();
            reader.read_line(&mut result_line).await.unwrap();
            let result_json: Value = serde_json::from_str(result_line.trim()).unwrap();
            assert_eq!(result_json["type"], "tool_result");
            assert_eq!(result_json["toolCallId"], "uds-1");
            assert_eq!(result_json["content"], "sent");
            assert_eq!(result_json["isError"], false);
        });

        let tools = vec![McpTool {
            name: "community.chat.send_dm".into(),
            description: "Send a DM".into(),
            input_schema: serde_json::json!({"type": "object"}),
        }];
        let registrations = build_registrations(&tools).unwrap();
        let mapping = build_mapping(&tools).unwrap();
        let mcp = McpClient::new(mcp_server.uri(), "token".into());
        serve_uds_extension(&socket, registrations, mapping, mcp)
            .await
            .unwrap();
        server_task.await.unwrap();
    }
}
