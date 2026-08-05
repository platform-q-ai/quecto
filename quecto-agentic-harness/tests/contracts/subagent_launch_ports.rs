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
