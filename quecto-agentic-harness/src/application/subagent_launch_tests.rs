use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::application::subagent_launch::{
    LaunchFuture, LaunchIdentity, PreparedRuntime, RegisteredLaunch, SubagentLaunchPorts,
    SubagentLaunchUseCase,
};
use crate::domain::error::DomainError;
use crate::domain::subagent::{ContainerSelection, SubagentConfig};
use crate::domain::tool::ToolResult;

#[derive(Debug, Default)]
struct RecordingPorts {
    events: Arc<Mutex<Vec<String>>>,
    fail_ready: bool,
    fail_register: bool,
    fail_initial_prompt: bool,
}

impl RecordingPorts {
    fn events(&self) -> Arc<Mutex<Vec<String>>> {
        self.events.clone()
    }
    fn record(&self, event: impl Into<String>) {
        self.events.lock().unwrap().push(event.into());
    }
}

impl SubagentLaunchPorts for RecordingPorts {
    type Prepared = bool;

    fn allocate_identity(
        &mut self,
        _config: &SubagentConfig,
    ) -> Result<LaunchIdentity, DomainError> {
        self.record("identity");
        Ok(LaunchIdentity {
            session_name: "child".into(),
            registry_key: "uuid".into(),
        })
    }

    fn build_cli_args(
        &mut self,
        _identity: &LaunchIdentity,
        _config: &SubagentConfig,
    ) -> Result<Vec<String>, DomainError> {
        self.record("cli");
        Ok(vec!["agent".into()])
    }

    fn resolve_binary(&mut self) -> Result<PathBuf, DomainError> {
        self.record("binary");
        Ok(PathBuf::from("quecto"))
    }

    fn prepare_child<'a>(
        &'a mut self,
        _config: &'a SubagentConfig,
        _binary: &'a Path,
        _cli_args: &'a [String],
    ) -> LaunchFuture<'a, Result<Self::Prepared, DomainError>> {
        self.record("prepare");
        Box::pin(async { Ok(true) })
    }

    fn ready<'a>(
        &'a mut self,
        _prepared: &'a mut Self::Prepared,
    ) -> LaunchFuture<'a, Result<PreparedRuntime, DomainError>> {
        self.record("ready");
        let fail = self.fail_ready;
        Box::pin(async move {
            if fail {
                Err(DomainError::Tool("ready failed".into()))
            } else {
                Ok(PreparedRuntime {
                    socket_path: PathBuf::from("/tmp/child.sock"),
                    pid: 7,
                    environment_ref: Some("env".into()),
                })
            }
        })
    }

    fn rollback_prepared<'a>(
        &'a mut self,
        _prepared: &'a mut Self::Prepared,
    ) -> LaunchFuture<'a, ()> {
        self.record("rollback-prepared");
        Box::pin(async {})
    }

    fn cleanup_registered_once<'a>(&'a mut self, registry_key: &'a str) -> LaunchFuture<'a, ()> {
        self.record(format!("cleanup-registered:{registry_key}"));
        Box::pin(async {})
    }

    fn register_and_monitor<'a>(
        &'a mut self,
        _identity: &'a LaunchIdentity,
        runtime: PreparedRuntime,
        _prepared: &'a mut Self::Prepared,
        _config: &'a SubagentConfig,
    ) -> LaunchFuture<'a, Result<RegisteredLaunch, DomainError>> {
        self.record("register");
        let fail = self.fail_register;
        Box::pin(async move {
            if fail {
                Err(DomainError::Tool("register failed".into()))
            } else {
                Ok(RegisteredLaunch {
                    registry_key: "uuid".into(),
                    socket_path: runtime.socket_path,
                })
            }
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
                Err(DomainError::Tool("prompt failed".into()))
            } else {
                Ok(())
            }
        })
    }

    fn unregister(&mut self, registry_key: &str) {
        self.record(format!("unregister:{registry_key}"));
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

fn config_with_task(task: Option<&str>) -> SubagentConfig {
    SubagentConfig {
        task: task.map(str::to_string),
        container: ContainerSelection::Local,
        agent_id: Some("child".into()),
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
async fn launch_use_case_rolls_back_prepared_child_on_register_failure() {
    let ports = RecordingPorts {
        fail_register: true,
        ..Default::default()
    };
    let events = ports.events();

    let err = SubagentLaunchUseCase::new(ports)
        .execute(&config_with_task(None))
        .await
        .unwrap_err();

    assert!(err.to_string().contains("register failed"));
    assert_eq!(
        events.lock().unwrap().as_slice(),
        [
            "identity",
            "cli",
            "binary",
            "prepare",
            "ready",
            "register",
            "rollback-prepared"
        ]
    );
}

#[tokio::test]
async fn launch_use_case_cleans_registered_child_on_initial_prompt_failure() {
    let ports = RecordingPorts {
        fail_initial_prompt: true,
        ..Default::default()
    };
    let events = ports.events();

    let err = SubagentLaunchUseCase::new(ports)
        .execute(&config_with_task(Some("hello")))
        .await
        .unwrap_err();

    assert!(err.to_string().contains("prompt failed"));
    assert_eq!(
        events.lock().unwrap().as_slice(),
        [
            "identity",
            "cli",
            "binary",
            "prepare",
            "ready",
            "register",
            "initial-prompt",
            "cleanup-registered:uuid",
            "unregister:uuid"
        ]
    );
}
