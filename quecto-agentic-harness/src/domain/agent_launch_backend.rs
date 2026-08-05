use crate::domain::container_runtime::{
    ContainerScriptsConfig, ExistingContainerRef, SpawnContainerRequest,
};
use crate::domain::error::DomainError;
use serde_json::Value;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Stdio;

pub type LaunchFuture<'a> =
    Pin<Box<dyn Future<Output = Result<PreparedAgentLaunch, DomainError>> + Send + 'a>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParentEndpoint {
    DirectUds(PathBuf),
    Proxy(String),
}

fn parse_proxy_unix_path(proxy: &str) -> Option<PathBuf> {
    proxy
        .strip_prefix("unix://")
        .or_else(|| proxy.strip_prefix("unix:"))
        .map(PathBuf::from)
}

pub fn validate_socket_proxy(proxy: &str, script_name: &str) -> Result<(), DomainError> {
    let Some(path) = parse_proxy_unix_path(proxy) else {
        return Err(DomainError::Tool(format!(
            "container script '{script_name}' socket_proxy must use unix:<absolute-path>"
        )));
    };
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(DomainError::Tool(format!(
            "container script '{script_name}' socket_proxy must be an absolute unix socket path without '..'"
        )));
    }
    Ok(())
}

impl ParentEndpoint {
    pub fn proxy_unix_path(&self) -> Option<PathBuf> {
        match self {
            Self::Proxy(proxy) => parse_proxy_unix_path(proxy),
            Self::DirectUds(_) => None,
        }
    }
    pub fn socket_path(&self) -> Option<&Path> {
        match self {
            Self::DirectUds(p) => Some(p),
            Self::Proxy(_) => None,
        }
    }
    pub fn mode(&self) -> &'static str {
        match self {
            Self::DirectUds(_) => "direct",
            Self::Proxy(_) => "proxy",
        }
    }
}

#[derive(Debug)]
pub struct PreparedAgentLaunch {
    /// Process owned by Quecto. Script-managed container launches return None
    /// because create/exec scripts are authoritative: they create/join the
    /// runtime and start the child agent exactly once.
    pub command: Option<tokio::process::Command>,
    pub backend_name: String,
    pub container: Option<ContainerLaunchOutcome>,
    pub endpoint: ParentEndpoint,
}

pub trait AgentLaunchBackend: Send + Sync {
    fn backend_name(&self) -> &'static str;
    fn can_launch(&self, request: &SpawnContainerRequest) -> bool;
    fn build_exec_command(&self) -> Option<&str> {
        None
    }
    fn prepare_launch<'a>(&'a self, spec: AgentLaunchSpec<'a>) -> LaunchFuture<'a>;
}

pub struct RetainedContainerScript<'a> {
    pub environment_id: &'a str,
    pub script_name: &'a str,
    pub exec_command: &'a str,
    pub inspect_command: &'a str,
    pub kill_command: &'a str,
}

pub struct AgentLaunchSpec<'a> {
    pub request: &'a SpawnContainerRequest,
    pub agent_uuid: &'a str,
    pub parent_agent_uuid: Option<&'a str>,
    pub child_binary: &'a Path,
    pub child_args: &'a [std::ffi::OsString],
    pub requested_socket_path: &'a Path,
    pub read_only: bool,
    pub existing_environment_id: Option<&'a str>,
    pub retained_container_script: Option<RetainedContainerScript<'a>>,
}

#[derive(Debug, Default)]
pub struct LocalProcessLaunchBackend;
impl AgentLaunchBackend for LocalProcessLaunchBackend {
    fn backend_name(&self) -> &'static str {
        "local"
    }
    fn can_launch(&self, request: &SpawnContainerRequest) -> bool {
        matches!(request, SpawnContainerRequest::Local)
    }
    fn prepare_launch<'a>(&'a self, spec: AgentLaunchSpec<'a>) -> LaunchFuture<'a> {
        Box::pin(async move {
            let mut c = tokio::process::Command::new(spec.child_binary);
            c.args(spec.child_args);
            Ok(PreparedAgentLaunch {
                command: Some(c),
                backend_name: self.backend_name().into(),
                container: None,
                endpoint: ParentEndpoint::DirectUds(spec.requested_socket_path.to_path_buf()),
            })
        })
    }
}

#[derive(Debug, Clone)]
pub struct ScriptManagedContainerLaunchBackend {
    config: ContainerScriptsConfig,
    parent_repo: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerLaunchOutcome {
    pub environment_id: String,
    pub socket_path: Option<PathBuf>,
    pub socket_proxy: Option<String>,
    pub workspace_path: PathBuf,
    pub metadata: Value,
    pub repository: Option<String>,
    pub script_name: String,
    pub container_ref: Option<String>,
    pub container_id: Option<String>,
    pub container_name: Option<String>,
    pub status: Option<String>,
    pub exec_command: String,
    pub inspect_command: String,
    pub kill_command: String,
}
impl Default for ScriptManagedContainerLaunchBackend {
    fn default() -> Self {
        Self::new(ContainerScriptsConfig::default(), None)
    }
}
impl ScriptManagedContainerLaunchBackend {
    pub fn new(config: ContainerScriptsConfig, parent_repo: Option<String>) -> Self {
        Self {
            config,
            parent_repo,
        }
    }
    fn resolve_new_request<'a>(
        &'a self,
        request: &'a SpawnContainerRequest,
    ) -> Result<
        (
            &'a str,
            &'a crate::domain::container_runtime::ContainerScriptSet,
            Option<String>,
        ),
        DomainError,
    > {
        let Some((n, s)) = request
            .resolve_script(&self.config)
            .map_err(DomainError::Tool)?
        else {
            return Err(DomainError::Tool(
                "container backend requires a new-container request".into(),
            ));
        };
        let repo = match request {
            SpawnContainerRequest::New { repo, .. } => {
                repo.clone().or_else(|| self.parent_repo.clone())
            }
            _ => None,
        };
        Ok((n, s, repo))
    }
    async fn run_script(
        &self,
        params: ScriptRun<'_>,
    ) -> Result<ContainerLaunchOutcome, DomainError> {
        let argv = parse_configured_script_command(params.script)?;
        let mut cmd = tokio::process::Command::new(&argv[0]);
        cmd.args(&argv[1..]);
        cmd.args(params.args).envs(params.envs);
        if let Some(r) = &params.repo {
            cmd.env("QUECTO_REPO_URL", r);
        }
        let out = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .output()
            .await
            .map_err(|e| {
                DomainError::Tool(format!(
                    "failed to run container script '{}': {e}",
                    params.script_name
                ))
            })?;
        if !out.status.success() {
            return Err(DomainError::Tool(format!(
                "container script '{}' failed with status {}",
                params.script_name, out.status
            )));
        }
        parse_launch_output(params.script_name, params.repo, &out.stdout, params.set)
    }
}

struct ScriptRun<'a> {
    script_name: &'a str,
    script: &'a str,
    repo: Option<String>,
    args: Vec<std::ffi::OsString>,
    envs: Vec<(&'a str, String)>,
    set: &'a crate::domain::container_runtime::ContainerScriptSet,
}

fn parse_configured_script_command(command: &str) -> Result<Vec<String>, DomainError> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut chars = command.chars().peekable();
    let mut quote: Option<char> = None;
    while let Some(ch) = chars.next() {
        match (quote, ch) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), '\\') => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            (Some(_), c) => current.push(c),
            (None, '\'' | '"') => quote = Some(ch),
            (None, c) if c.is_whitespace() => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
            }
            (None, '\\') => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            (
                None,
                c @ (';' | '|' | '&' | '$' | '`' | '<' | '>' | '*' | '?' | '{' | '}' | '(' | ')'),
            ) => {
                return Err(DomainError::Tool(format!(
                    "shell metacharacter '{c}' is not allowed; configure an executable path plus arguments"
                )));
            }
            (None, c) if c.is_control() && c != '\t' => {
                return Err(DomainError::Tool(
                    "control characters are not allowed".into(),
                ));
            }
            (None, c) => current.push(c),
        }
    }
    if quote.is_some() {
        return Err(DomainError::Tool(
            "unterminated quote in container script command".into(),
        ));
    }
    if !current.is_empty() {
        args.push(current);
    }
    if args.is_empty() {
        return Err(DomainError::Tool(
            "container script command is empty".into(),
        ));
    }
    Ok(args)
}

fn reject_repo(repo: Option<&str>) -> Result<(), DomainError> {
    if let Some(r) = repo {
        if r.starts_with('-') || r.chars().any(|c| c.is_control()) {
            return Err(DomainError::Tool(
                "repository URL must not start with '-' or contain control characters".into(),
            ));
        }
    }
    Ok(())
}
fn parse_launch_output(
    script_name: &str,
    repo: Option<String>,
    stdout: &[u8],
    set: &crate::domain::container_runtime::ContainerScriptSet,
) -> Result<ContainerLaunchOutcome, DomainError> {
    let v: Value = serde_json::from_slice(stdout).map_err(|e| {
        DomainError::Tool(format!(
            "container script '{script_name}' did not return JSON: {e}"
        ))
    })?;
    let sf = |n: &str| v.get(n).and_then(Value::as_str).map(str::to_string);
    let environment_id = sf("environment_id").ok_or_else(|| {
        DomainError::Tool(format!(
            "container script '{script_name}' output missing string field 'environment_id'"
        ))
    })?;
    let _container_ref = sf("container_ref").ok_or_else(|| {
        DomainError::Tool(format!(
            "container script '{script_name}' output missing string field 'container_ref'"
        ))
    })?;
    let workspace_path = PathBuf::from(sf("workspace_path").ok_or_else(|| {
        DomainError::Tool(format!(
            "container script '{script_name}' output missing string field 'workspace_path'"
        ))
    })?);
    let metadata = v.get("metadata").cloned().ok_or_else(|| {
        DomainError::Tool(format!(
            "container script '{script_name}' output missing required object field 'metadata'"
        ))
    })?;
    if !metadata.is_object() {
        return Err(DomainError::Tool(format!(
            "container script '{script_name}' field 'metadata' must be an object"
        )));
    }
    let socket_path = sf("socket_path").map(PathBuf::from);
    let socket_proxy = sf("socket_proxy");
    match (socket_path.is_some(), socket_proxy.is_some()) {
        (true, false) => {}
        (false, true) => validate_socket_proxy(socket_proxy.as_deref().unwrap(), script_name)?,
        (false, false) => {
            return Err(DomainError::Tool(format!(
                "container script '{script_name}' output must include exactly one endpoint: socket_path or socket_proxy"
            )));
        }
        (true, true) => {
            return Err(DomainError::Tool(format!(
                "container script '{script_name}' output must not include both socket_path and socket_proxy"
            )));
        }
    }
    Ok(ContainerLaunchOutcome {
        environment_id,
        socket_path,
        socket_proxy,
        workspace_path,
        metadata,
        repository: repo,
        script_name: script_name.into(),
        container_ref: sf("container_ref"),
        container_id: sf("container_id"),
        container_name: sf("container_name"),
        status: sf("status"),
        exec_command: set.exec.clone(),
        inspect_command: set.inspect.clone(),
        kill_command: set.kill.clone(),
    })
}
impl AgentLaunchBackend for ScriptManagedContainerLaunchBackend {
    fn backend_name(&self) -> &'static str {
        "container-script"
    }
    fn can_launch(&self, r: &SpawnContainerRequest) -> bool {
        matches!(
            r,
            SpawnContainerRequest::New { .. } | SpawnContainerRequest::Existing { .. }
        )
    }
    fn build_exec_command(&self) -> Option<&str> {
        Some("script-managed-container")
    }
    fn prepare_launch<'a>(&'a self, spec: AgentLaunchSpec<'a>) -> LaunchFuture<'a> {
        Box::pin(async move {
            match spec.request {
                SpawnContainerRequest::New { .. } => {
                    let (name, set, repo) = self.resolve_new_request(spec.request)?;
                    reject_repo(repo.as_deref())?;
                    let mut args = vec![
                        "--script-name".into(),
                        name.into(),
                        "--socket-path".into(),
                        spec.requested_socket_path.as_os_str().to_os_string(),
                        "--read-only".into(),
                        (if spec.read_only { "true" } else { "false" }).into(),
                    ];
                    if let Some(r) = &repo {
                        args.push("--repo".into());
                        args.push(r.into());
                    }
                    args.push("--".into());
                    args.push(spec.child_binary.as_os_str().to_os_string());
                    args.extend(spec.child_args.iter().cloned());
                    let out = self
                        .run_script(ScriptRun {
                            script_name: name,
                            script: &set.create,
                            repo,
                            args,
                            envs: vec![
                                ("QUECTO_AGENT_UUID", spec.agent_uuid.into()),
                                ("QUECTO_ENVIRONMENT_UUID", spec.agent_uuid.into()),
                            ],
                            set,
                        })
                        .await?;
                    let endpoint = out
                        .socket_proxy
                        .as_ref()
                        .map(|p| ParentEndpoint::Proxy(p.clone()))
                        .unwrap_or_else(|| {
                            ParentEndpoint::DirectUds(
                                out.socket_path
                                    .clone()
                                    .unwrap_or_else(|| spec.requested_socket_path.to_path_buf()),
                            )
                        });
                    Ok(PreparedAgentLaunch {
                        command: None,
                        backend_name: self.backend_name().into(),
                        container: Some(out),
                        endpoint,
                    })
                }
                SpawnContainerRequest::Existing { reference } => {
                    let retained = spec.retained_container_script.as_ref().ok_or_else(|| {
                        DomainError::Tool(
                            "existing container join missing retained script authority".into(),
                        )
                    })?;
                    let name = retained.script_name;
                    let set = crate::domain::container_runtime::ContainerScriptSet {
                        create: String::new(),
                        exec: retained.exec_command.to_string(),
                        inspect: retained.inspect_command.to_string(),
                        kill: retained.kill_command.to_string(),
                    };
                    let env_id = retained.environment_id;
                    let mut args = vec![
                        "--script-name".into(),
                        name.into(),
                        "--environment-id".into(),
                        env_id.into(),
                        "--socket-path".into(),
                        spec.requested_socket_path.as_os_str().to_os_string(),
                        "--read-only".into(),
                        (if spec.read_only { "true" } else { "false" }).into(),
                    ];
                    match reference {
                        ExistingContainerRef::Ref(r) => {
                            args.push("--container-ref".into());
                            args.push(r.into())
                        }
                        ExistingContainerRef::Name(n) => {
                            args.push("--container-name".into());
                            args.push(n.into())
                        }
                    };
                    args.push("--".into());
                    args.push(spec.child_binary.as_os_str().to_os_string());
                    args.extend(spec.child_args.iter().cloned());
                    let out = self
                        .run_script(ScriptRun {
                            script_name: name,
                            script: &set.exec,
                            repo: None,
                            args,
                            envs: vec![
                                ("QUECTO_AGENT_UUID", spec.agent_uuid.into()),
                                ("QUECTO_ENVIRONMENT_UUID", env_id.into()),
                            ],
                            set: &set,
                        })
                        .await?;
                    let endpoint = out
                        .socket_proxy
                        .as_ref()
                        .map(|p| ParentEndpoint::Proxy(p.clone()))
                        .unwrap_or_else(|| {
                            ParentEndpoint::DirectUds(
                                out.socket_path
                                    .clone()
                                    .unwrap_or_else(|| spec.requested_socket_path.to_path_buf()),
                            )
                        });
                    Ok(PreparedAgentLaunch {
                        command: None,
                        backend_name: self.backend_name().into(),
                        container: Some(out),
                        endpoint,
                    })
                }
                SpawnContainerRequest::Local => Err(DomainError::Tool(
                    "container backend cannot launch local request".into(),
                )),
            }
        })
    }
}
