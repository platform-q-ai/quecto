use crate::{QuectoMcpError, Result};
use std::fs::File;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone)]
pub struct Config {
    pub socket: PathBuf,
    pub mcp_url: String,
    pub mcp_token: String,
    pub mcp_server_name: String,
    pub tool_prefixes: Vec<String>,
    pub tool_allowlist: Vec<String>,
    pub tool_denylist: Vec<String>,
    pub name_prefix: String,
    pub timeout: Duration,
    pub register_timeout: Duration,
    pub refresh_interval: Option<Duration>,
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
        let mut name_prefix = String::new();
        let mut timeout = std::env::var("QUECTO_MCP_TIMEOUT_SECONDS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or_else(|| Duration::from_secs(30));
        let mut register_timeout = Duration::from_secs(10);
        let refresh_interval = None;

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
                "--name-prefix" => name_prefix = iter.next().unwrap_or_default(),
                "--refresh-interval" => {
                    let _ = iter.next();
                    return Err(QuectoMcpError::UnsupportedOption(
                        "--refresh-interval is not implemented; restart quecto-mcp to refresh tool registrations".into(),
                    ));
                }
                "--register-timeout" => {
                    if let Some(value) = iter.next().and_then(|s| s.parse::<u64>().ok()) {
                        register_timeout = Duration::from_secs(value);
                    }
                }
                "--mcp-token-file" => {
                    let path = iter.next().ok_or_else(|| {
                        QuectoMcpError::InvalidTokenSource(
                            "--mcp-token-file requires a path".into(),
                        )
                    })?;
                    mcp_token = Some(read_token_file(&path)?);
                }
                "--mcp-token-command" => {
                    let command = iter.next().ok_or_else(|| {
                        QuectoMcpError::InvalidTokenSource(
                            "--mcp-token-command requires a command".into(),
                        )
                    })?;
                    mcp_token = Some(run_token_command(&command)?);
                }
                "--timeout" => {
                    if let Some(value) = iter.next().and_then(|s| s.parse::<u64>().ok()) {
                        timeout = Duration::from_secs(value);
                    }
                }
                _ => {}
            }
        }

        if tool_prefixes.is_empty() && allowlist.is_empty() {
            tool_prefixes.push("community.".to_string());
        }

        Ok(Self {
            socket: socket
                .ok_or_else(|| QuectoMcpError::Quecto("missing --socket / QUECTO_SOCKET".into()))?,
            mcp_url: mcp_url
                .ok_or_else(|| QuectoMcpError::Mcp("missing --mcp-url / PERME8_MCP_URL".into()))?,
            mcp_token: validate_token(mcp_token.ok_or_else(|| {
                QuectoMcpError::Mcp("missing --mcp-token / PERME8_MCP_TOKEN".into())
            })?)?,
            mcp_server_name: server_name,
            tool_prefixes,
            tool_allowlist: allowlist,
            tool_denylist: denylist,
            name_prefix,
            timeout,
            register_timeout,
            refresh_interval,
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

fn read_token_file(path: &str) -> Result<String> {
    let value = std::fs::read_to_string(path).map_err(|err| {
        QuectoMcpError::InvalidTokenSource(format!("--mcp-token-file {path}: {err}"))
    })?;
    validate_token(value)
}

fn validate_token(token: String) -> Result<String> {
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err(QuectoMcpError::InvalidTokenSource("empty MCP token".into()));
    }
    Ok(token)
}

const TOKEN_COMMAND_TIMEOUT: Duration = Duration::from_secs(10);

fn run_token_command(command: &str) -> Result<String> {
    let argv = shlex::split(command).ok_or_else(|| {
        QuectoMcpError::InvalidTokenSource(
            "--mcp-token-command has invalid shell-style quoting".into(),
        )
    })?;
    let Some((program, args)) = argv.split_first() else {
        return Err(QuectoMcpError::InvalidTokenSource(
            "--mcp-token-command is empty".into(),
        ));
    };
    let output = run_command_with_timeout(program, args, TOKEN_COMMAND_TIMEOUT)?;
    if !output.status.success() {
        return Err(QuectoMcpError::InvalidTokenSource(format!(
            "--mcp-token-command exited with {}",
            output.status
        )));
    }
    validate_token(String::from_utf8_lossy(&output.stdout).to_string())
}

fn run_command_with_timeout(
    program: &str,
    args: &[String],
    timeout: Duration,
) -> Result<std::process::Output> {
    let output_path = std::env::temp_dir().join(format!(
        "quecto-mcp-token-{}-{}.out",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let mut child = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .stdout(File::create(&output_path).map_err(|err| {
            QuectoMcpError::InvalidTokenSource(format!("--mcp-token-command: {err}"))
        })?)
        .spawn()
        .map_err(|err| QuectoMcpError::InvalidTokenSource(format!("--mcp-token-command: {err}")))?;
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|err| {
            QuectoMcpError::InvalidTokenSource(format!("--mcp-token-command: {err}"))
        })? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&output_path);
            return Err(QuectoMcpError::InvalidTokenSource(format!(
                "--mcp-token-command timed out after {}s",
                timeout.as_secs()
            )));
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    let stdout = std::fs::read(&output_path)
        .map_err(|err| QuectoMcpError::InvalidTokenSource(format!("--mcp-token-command: {err}")))?;
    let _ = std::fs::remove_file(output_path);
    Ok(std::process::Output {
        status,
        stdout,
        stderr: Vec::new(),
    })
}
