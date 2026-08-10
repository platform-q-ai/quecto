use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use quecto::application::subagent_launch::{
    LaunchFuture, LaunchIdentity, PreparedRuntime, RegisteredLaunch, SubagentLaunchPorts,
    SubagentLaunchUseCase,
};
use quecto::domain::error::DomainError;
use quecto::domain::subagent::{ContainerSelection, SubagentConfig};
use quecto::domain::tool::ToolResult;

#[derive(Default)]
struct ContractPorts {
    events: Arc<Mutex<Vec<String>>>,
    fail_initial_prompt: bool,
}

impl ContractPorts {
    fn events(&self) -> Arc<Mutex<Vec<String>>> {
        self.events.clone()
    }
    fn record(&self, event: impl Into<String>) {
        self.events.lock().unwrap().push(event.into());
    }
}

impl SubagentLaunchPorts for ContractPorts {
    type Prepared = ();

    fn initial_prompt_retry_deadline(&self) -> Option<tokio::time::Instant> {
        None
    }

    fn allocate_identity(
        &mut self,
        _config: &SubagentConfig,
    ) -> Result<LaunchIdentity, DomainError> {
        self.record("identity");
        Ok(LaunchIdentity {
            session_name: "contract-child".into(),
            registry_key: "contract-uuid".into(),
        })
    }

    fn build_cli_args(
        &mut self,
        _identity: &LaunchIdentity,
        _config: &SubagentConfig,
    ) -> Result<Vec<std::ffi::OsString>, DomainError> {
        self.record("cli");
        Ok(vec![])
    }

    fn resolve_binary(&mut self) -> Result<PathBuf, DomainError> {
        self.record("binary");
        Ok(PathBuf::from("quecto"))
    }

    fn prepare_child<'a>(
        &'a mut self,
        _config: &'a SubagentConfig,
        _binary: &'a Path,
        _cli_args: &'a [std::ffi::OsString],
    ) -> LaunchFuture<'a, Result<Self::Prepared, DomainError>> {
        self.record("prepare");
        Box::pin(async { Ok(()) })
    }

    fn ready<'a>(
        &'a mut self,
        _prepared: &'a mut Self::Prepared,
    ) -> LaunchFuture<'a, Result<PreparedRuntime, DomainError>> {
        self.record("ready");
        Box::pin(async {
            Ok(PreparedRuntime {
                socket_path: PathBuf::from("/tmp/contract.sock"),
                pid: 0,
                environment_ref: None,
            })
        })
    }

    fn rollback_prepared<'a>(
        &'a mut self,
        _prepared: &'a mut Self::Prepared,
    ) -> LaunchFuture<'a, ()> {
        self.record("rollback-prepared");
        Box::pin(async {})
    }

    fn uncommit_registered<'a>(&'a mut self, registry_key: &'a str) -> LaunchFuture<'a, ()> {
        self.record(format!("uncommit:{registry_key}"));
        Box::pin(async {})
    }

    fn register_and_monitor<'a>(
        &'a mut self,
        _identity: &'a LaunchIdentity,
        runtime: PreparedRuntime,
        _prepared: &'a mut Self::Prepared,
        _config: &'a SubagentConfig,
    ) -> LaunchFuture<'a, Result<RegisteredLaunch, DomainError>> {
        self.record("register-monitor");
        Box::pin(async move {
            Ok(RegisteredLaunch {
                registry_key: "contract-uuid".into(),
                socket_path: runtime.socket_path,
            })
        })
    }

    fn send_initial_prompt<'a>(
        &'a mut self,
        _socket_path: &'a Path,
        _task: &'a str,
        _deadline: Option<tokio::time::Instant>,
    ) -> LaunchFuture<'a, Result<(), DomainError>> {
        self.record("initial-prompt");
        let fail = self.fail_initial_prompt;
        Box::pin(async move {
            if fail {
                Err(DomainError::Tool("initial prompt failed".into()))
            } else {
                Ok(())
            }
        })
    }

    fn success(&self, _identity: &LaunchIdentity, _environment_ref: Option<&str>) -> ToolResult {
        self.record("success");
        ToolResult {
            content: "ok".into(),
            is_error: false,
            image_blocks: vec![],
        }
    }
}

fn config(task: Option<&str>) -> SubagentConfig {
    SubagentConfig {
        task: task.map(str::to_string),
        container: ContainerSelection::Local,
        agent_id: Some("contract-child".into()),
        restrict_to_workspace: false,
        system: None,
        config_path: None,
        workflow: false,
        workflow_guards: false,
        workflow_spec: None,
        model: None,
        effort: None,
        disable_tools: Vec::new(),
        read_only: false,
    }
}

/// Shared behavioral contract run against the REAL launch adapters (local
/// process and script-managed create) through the production
/// `SpawnLaunchPorts` implementation, not fakes.
mod real_adapters {
    use std::path::{Path, PathBuf};

    use quecto::application::subagent_launch::SubagentLaunchUseCase;
    use quecto::domain::environment_registry::EnvironmentRegistry;
    use quecto::domain::error::DomainError;
    use quecto::domain::subagent::ContainerSelection;
    use quecto::domain::subagent::SubagentConfig;
    use quecto::domain::tool::ToolResult;
    use quecto::infrastructure::tools::spawn::SpawnTool;

    /// Serializes scenarios that set `QUECTO_CHILD_BINARY`.
    static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    fn write_exec(path: &Path, content: &str) {
        std::fs::write(path, content).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut p = std::fs::metadata(path).unwrap().permissions();
            p.set_mode(0o700);
            std::fs::set_permissions(path, p).unwrap();
        }
    }

    /// A stand-in child: binds the UDS socket passed via `--socket` and holds
    /// accepted connections open so readiness and the monitor stay connected.
    const LISTENER: &str = r#"#!/usr/bin/env bash
set -euo pipefail
sock=""
prev=""
for a in "$@"; do
  if [ "$prev" = "--socket" ]; then sock="$a"; fi
  prev="$a"
done
exec python3 - "$sock" <<'PY'
import socket, sys
s = socket.socket(socket.AF_UNIX)
s.bind(sys.argv[1])
s.listen(8)
s.settimeout(15)
conns = []
try:
    while True:
        c, _ = s.accept()
        conns.append(c)
except Exception:
    pass
PY
"#;

    fn tool(dir: &Path) -> SpawnTool {
        SpawnTool::with_base_dir(Vec::new(), false, dir.to_path_buf())
            .with_socket_dir(dir.to_path_buf())
    }

    fn config(container: ContainerSelection, config_path: Option<PathBuf>) -> SubagentConfig {
        SubagentConfig {
            task: None,
            container,
            agent_id: Some("contract-adapter-child".into()),
            restrict_to_workspace: false,
            system: None,
            config_path,
            workflow: false,
            workflow_guards: false,
            workflow_spec: None,
            model: None,
            effort: None,
            disable_tools: Vec::new(),
            read_only: false,
        }
    }

    async fn run_launch(
        tool: &SpawnTool,
        config: &SubagentConfig,
    ) -> Result<ToolResult, DomainError> {
        SubagentLaunchUseCase::new(tool.launch_ports_for_contract())
            .execute(config)
            .await
    }

    fn committed_pids(tool: &SpawnTool) -> Vec<u32> {
        let entries = tool.registry().lock().unwrap();
        entries.values().map(|entry| entry.pid).collect()
    }

    #[tokio::test]
    async fn local_adapter_commits_one_entry_owning_the_child_process() {
        let _env = ENV_LOCK.lock().await;
        let dir = tempfile::TempDir::new().unwrap();
        let child = dir.path().join("child.sh");
        write_exec(&child, LISTENER);
        // SAFETY: ENV_LOCK serializes every scenario touching this process-wide override, which is set before any child process is launched.
        unsafe { std::env::set_var("QUECTO_CHILD_BINARY", &child) };

        let tool = tool(dir.path());
        let result = run_launch(&tool, &config(ContainerSelection::Local, None))
            .await
            .unwrap();
        // SAFETY: paired cleanup for the test-scoped environment override above, under ENV_LOCK.
        unsafe { std::env::remove_var("QUECTO_CHILD_BINARY") };

        assert!(!result.is_error, "{}", result.content);
        assert!(!result.content.contains("environment_ref="));
        let pids = committed_pids(&tool);
        assert_eq!(pids.len(), 1);
        assert_ne!(pids[0], 0, "local adapter must record the child it spawned");
    }

    #[tokio::test]
    async fn local_adapter_readiness_failure_rolls_back_to_no_committed_entry() {
        let _env = ENV_LOCK.lock().await;
        let dir = tempfile::TempDir::new().unwrap();
        let child = dir.path().join("child.sh");
        write_exec(&child, "#!/usr/bin/env bash\nexit 0\n");
        // SAFETY: ENV_LOCK serializes every scenario touching this process-wide override, which is set before any child process is launched.
        unsafe { std::env::set_var("QUECTO_CHILD_BINARY", &child) };

        let tool = tool(dir.path());
        let err = run_launch(&tool, &config(ContainerSelection::Local, None))
            .await
            .unwrap_err();
        // SAFETY: paired cleanup for the test-scoped environment override above, under ENV_LOCK.
        unsafe { std::env::remove_var("QUECTO_CHILD_BINARY") };

        assert!(
            err.to_string().contains("exited before socket ready"),
            "{err}"
        );
        assert!(committed_pids(&tool).is_empty());
    }

    fn script_fixture(dir: &Path, create_body: &str) -> PathBuf {
        let create = dir.join("create.sh");
        write_exec(&create, create_body);
        let cleanup = dir.join("cleanup.sh");
        write_exec(&cleanup, "#!/usr/bin/env bash\nexit 0\n");
        let cfg = dir.join("config.json");
        std::fs::write(
            &cfg,
            serde_json::json!({
                "container_configs": {
                    "default": {
                        "default": true,
                        "create": [create.to_string_lossy()],
                        "cleanup": [cleanup.to_string_lossy()],
                    },
                }
            })
            .to_string(),
        )
        .unwrap();
        cfg
    }

    fn script_container() -> ContainerSelection {
        ContainerSelection::New {
            container_config: None,
            name: None,
        }
    }

    #[tokio::test]
    async fn script_adapter_commits_environment_and_entry_without_local_child() {
        let _env = ENV_LOCK.lock().await;
        let dir = tempfile::TempDir::new().unwrap();
        let listener = dir.path().join("listener.sh");
        write_exec(&listener, LISTENER);
        let create_body = format!(
            r#"#!/usr/bin/env bash
set -euo pipefail
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--" ]; then shift; break; fi
  shift
done
'{listener}' "$@" >/dev/null 2>&1 &
sock=""
prev=""
for a in "$@"; do
  if [ "$prev" = "--socket" ]; then sock="$a"; fi
  prev="$a"
done
printf '{{"environment_id":"env-contract","workspace_path":"%s","metadata":{{}},"socket_path":"%s"}}' "$PWD" "$sock"
"#,
            listener = listener.display()
        );
        let cfg = script_fixture(dir.path(), &create_body);

        let environments = EnvironmentRegistry::new();
        let tool = tool(dir.path()).with_environment_registry(environments.clone());
        let result = run_launch(&tool, &config(script_container(), Some(cfg)))
            .await
            .unwrap();

        assert!(!result.is_error, "{}", result.content);
        assert!(result.content.contains("environment_ref=C1"));
        let pids = committed_pids(&tool);
        assert_eq!(pids.len(), 1);
        assert_eq!(pids[0], 0, "script adapter must not start a local child");
        let committed = environments.entries();
        assert_eq!(committed.len(), 1);
        assert_eq!(committed[0].environment_ref, "C1");
        assert_eq!(committed[0].environment_id, "env-contract");
        assert_eq!(committed[0].script_name, "default");
    }

    #[tokio::test]
    async fn script_adapter_create_failure_rolls_back_to_no_committed_entry() {
        let _env = ENV_LOCK.lock().await;
        let dir = tempfile::TempDir::new().unwrap();
        let cfg = script_fixture(dir.path(), "#!/usr/bin/env bash\nexit 1\n");

        let environments = EnvironmentRegistry::new();
        let tool = tool(dir.path()).with_environment_registry(environments.clone());
        let err = run_launch(&tool, &config(script_container(), Some(cfg)))
            .await
            .unwrap_err();

        assert!(err.to_string().contains("create failed"), "{err}");
        assert!(committed_pids(&tool).is_empty());
        assert!(environments.entries().is_empty());
        // The failed launch still consumed its ref: refs are never reused.
        assert_eq!(environments.mint_ref(), "C2");
    }
}

#[tokio::test]
async fn subagent_launch_port_contract_orders_register_before_initial_prompt_and_cleanup_after_prompt_failure()
 {
    let ports = ContractPorts {
        fail_initial_prompt: true,
        ..Default::default()
    };
    let events = ports.events();

    let err = SubagentLaunchUseCase::new(ports)
        .execute(&config(Some("hi")))
        .await
        .unwrap_err();

    assert!(err.to_string().contains("initial prompt failed"));
    assert_eq!(
        events.lock().unwrap().as_slice(),
        [
            "identity",
            "cli",
            "binary",
            "prepare",
            "ready",
            "register-monitor",
            "initial-prompt",
            "uncommit:contract-uuid"
        ]
    );
}
