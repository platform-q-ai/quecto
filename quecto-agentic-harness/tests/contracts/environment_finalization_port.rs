use std::sync::{Arc, Mutex};

use quecto::domain::environment_finalization::{
    EnvironmentFinalizationPort, EnvironmentFinalizationUseCase, MemberFinalizeMode,
};
use quecto::domain::environment_registry::{
    EnvironmentRecord, EnvironmentRegistry, EnvironmentStatus,
};
use quecto::domain::subagent_launch::LaunchFuture;

#[derive(Debug, PartialEq, Eq)]
struct ContractCall {
    operation: &'static str,
    environment_id: String,
    argv: Vec<String>,
}

#[derive(Default)]
struct ContractPort {
    calls: Mutex<Vec<ContractCall>>,
}

impl EnvironmentFinalizationPort for ContractPort {
    fn run_retained_inspect<'a>(
        &'a self,
        _environment_id: &'a str,
        _argv: &'a [String],
    ) -> LaunchFuture<'a, Result<serde_json::Value, String>> {
        Box::pin(async { Ok(serde_json::json!({})) })
    }

    fn run_retained_kill<'a>(
        &'a self,
        environment_id: &'a str,
        argv: &'a [String],
    ) -> LaunchFuture<'a, Result<(), String>> {
        Box::pin(async move {
            self.calls.lock().unwrap().push(ContractCall {
                operation: "kill",
                environment_id: environment_id.to_string(),
                argv: argv.to_vec(),
            });
            Ok(())
        })
    }

    fn run_retained_cleanup<'a>(
        &'a self,
        environment_id: &'a str,
        argv: &'a [String],
    ) -> LaunchFuture<'a, ()> {
        Box::pin(async move {
            self.calls.lock().unwrap().push(ContractCall {
                operation: "cleanup",
                environment_id: environment_id.to_string(),
                argv: argv.to_vec(),
            });
        })
    }
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    futures::executor::block_on(future)
}

#[test]
fn environment_finalization_port_contract_runs_only_the_claimed_teardown_script() {
    let registry = EnvironmentRegistry::new();
    let env_ref = registry.mint_ref();
    registry.commit(EnvironmentRecord {
        environment_ref: env_ref.clone(),
        environment_id: "contract-runtime".to_string(),
        environment_uuid: "contract-uuid".to_string(),
        name: None,
        workspace_path: std::path::PathBuf::from("/contract"),
        repository: "https://example.invalid/repo.git".to_string(),
        script_name: "default".to_string(),
        retained_exec_argv: vec!["exec.sh".to_string()],
        retained_kill_argv: vec!["kill.sh".to_string(), "--flag".to_string()],
        retained_cleanup_argv: vec!["cleanup.sh".to_string()],
        retained_inspect_argv: vec![],
        members: vec!["agent-a".to_string()],
        status: EnvironmentStatus::Running,
        metadata: serde_json::json!({}),
        last_error: None,
    });
    let port = Arc::new(ContractPort::default());
    let use_case = EnvironmentFinalizationUseCase::new(registry, port.clone());

    block_on(use_case.finalize_member(&env_ref, "agent-a", None, MemberFinalizeMode::Exit));

    assert_eq!(
        port.calls.lock().unwrap().as_slice(),
        &[ContractCall {
            operation: "kill",
            environment_id: "contract-runtime".to_string(),
            argv: vec!["kill.sh".to_string(), "--flag".to_string()],
        }]
    );
}
