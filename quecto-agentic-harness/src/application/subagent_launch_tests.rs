use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
    initial_prompt_retry_deadline: Option<tokio::time::Instant>,
    initial_prompt_failures_remaining: usize,
    initial_prompt_delay: Option<Duration>,
    observed_initial_prompt_deadlines: Arc<Mutex<Vec<Option<tokio::time::Instant>>>>,
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

    fn initial_prompt_retry_deadline(&self) -> Option<tokio::time::Instant> {
        self.initial_prompt_retry_deadline
    }

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
    ) -> Result<Vec<std::ffi::OsString>, DomainError> {
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
        _cli_args: &'a [std::ffi::OsString],
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

    fn uncommit_registered<'a>(&'a mut self, registry_key: &'a str) -> LaunchFuture<'a, ()> {
        self.record(format!("uncommit-registered:{registry_key}"));
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
        deadline: Option<tokio::time::Instant>,
    ) -> LaunchFuture<'a, Result<(), DomainError>> {
        self.record("initial-prompt");
        self.observed_initial_prompt_deadlines
            .lock()
            .unwrap()
            .push(deadline);
        let fail = self.fail_initial_prompt || self.initial_prompt_failures_remaining > 0;
        self.initial_prompt_failures_remaining =
            self.initial_prompt_failures_remaining.saturating_sub(1);
        let delay = self.initial_prompt_delay;
        Box::pin(async move {
            if let Some(delay) = delay {
                tokio::time::sleep(delay).await;
            }
            if fail {
                Err(DomainError::Tool("prompt failed".into()))
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

fn config_with_task(task: Option<&str>) -> SubagentConfig {
    SubagentConfig {
        task: task.map(str::to_string),
        container: ContainerSelection::Local,
        agent_id: Some("child".into()),
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
async fn launch_use_case_retries_initial_prompt_failure_when_endpoint_requests_readiness_retry() {
    let ports = RecordingPorts {
        initial_prompt_retry_deadline: Some(tokio::time::Instant::now() + Duration::from_secs(1)),
        initial_prompt_failures_remaining: 2,
        ..Default::default()
    };
    let events = ports.events();
    let observed_deadlines = ports.observed_initial_prompt_deadlines.clone();

    let result = SubagentLaunchUseCase::new(ports)
        .execute(&config_with_task(Some("hello")))
        .await
        .unwrap();

    assert!(!result.is_error);
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
            "initial-prompt",
            "initial-prompt",
            "success"
        ]
    );
    assert_eq!(observed_deadlines.lock().unwrap().len(), 3);
    assert!(
        observed_deadlines
            .lock()
            .unwrap()
            .iter()
            .all(Option::is_some),
        "every retried attempt must receive the shared readiness deadline"
    );
}

#[tokio::test]
async fn launch_use_case_does_not_start_initial_prompt_when_retry_deadline_already_expired() {
    let ports = RecordingPorts {
        initial_prompt_retry_deadline: Some(tokio::time::Instant::now() - Duration::from_millis(1)),
        ..Default::default()
    };
    let events = ports.events();
    let observed_deadlines = ports.observed_initial_prompt_deadlines.clone();

    let err = SubagentLaunchUseCase::new(ports)
        .execute(&config_with_task(Some("hello")))
        .await
        .unwrap_err();

    assert!(
        err.to_string()
            .contains("initial prompt retry deadline expired")
    );
    assert_eq!(observed_deadlines.lock().unwrap().len(), 0);
    assert_eq!(
        events.lock().unwrap().last().map(String::as_str),
        Some("uncommit-registered:uuid")
    );
}

#[tokio::test]
async fn launch_use_case_caps_initial_prompt_retry_sleep_to_remaining_budget() {
    let ports = RecordingPorts {
        initial_prompt_retry_deadline: Some(
            tokio::time::Instant::now() + Duration::from_millis(30),
        ),
        initial_prompt_failures_remaining: usize::MAX,
        ..Default::default()
    };
    let observed_deadlines = ports.observed_initial_prompt_deadlines.clone();
    let started = tokio::time::Instant::now();

    let err = SubagentLaunchUseCase::new(ports)
        .execute(&config_with_task(Some("hello")))
        .await
        .unwrap_err();

    assert!(
        err.to_string()
            .contains("initial prompt retry deadline expired")
    );
    assert_eq!(observed_deadlines.lock().unwrap().len(), 1);
    assert!(
        started.elapsed() < Duration::from_millis(80),
        "retry sleep should be capped to the remaining deadline budget, elapsed {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn launch_use_case_stops_prompt_retries_after_attempt_exhausts_budget() {
    let ports = RecordingPorts {
        initial_prompt_retry_deadline: Some(
            tokio::time::Instant::now() + Duration::from_millis(20),
        ),
        initial_prompt_delay: Some(Duration::from_millis(30)),
        initial_prompt_failures_remaining: usize::MAX,
        ..Default::default()
    };
    let observed_deadlines = ports.observed_initial_prompt_deadlines.clone();

    let err = SubagentLaunchUseCase::new(ports)
        .execute(&config_with_task(Some("hello")))
        .await
        .unwrap_err();

    assert!(err.to_string().contains("prompt failed"));
    assert_eq!(observed_deadlines.lock().unwrap().as_slice().len(), 1);
    assert!(observed_deadlines.lock().unwrap()[0].is_some());
}

#[tokio::test]
async fn launch_use_case_uncommits_registered_child_when_initial_prompt_retries_are_exhausted() {
    let ports = RecordingPorts {
        initial_prompt_retry_deadline: Some(
            tokio::time::Instant::now() + Duration::from_millis(150),
        ),
        initial_prompt_failures_remaining: usize::MAX,
        ..Default::default()
    };
    let events = ports.events();

    let err = SubagentLaunchUseCase::new(ports)
        .execute(&config_with_task(Some("hello")))
        .await
        .unwrap_err();

    assert!(
        err.to_string()
            .contains("initial prompt retry deadline expired")
    );
    assert_eq!(
        events
            .lock()
            .unwrap()
            .iter()
            .filter(|event| event.as_str() == "initial-prompt")
            .count(),
        2,
        "expected retries to stop when the next attempt would start after the deadline: {:?}",
        events.lock().unwrap().as_slice()
    );
    assert_eq!(
        events.lock().unwrap().last().map(String::as_str),
        Some("uncommit-registered:uuid")
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
            "uncommit-registered:uuid"
        ]
    );
}
