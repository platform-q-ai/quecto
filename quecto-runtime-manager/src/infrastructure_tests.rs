use super::infrastructure_k8s::*;
use super::*;
use crate::domain::{
    EnsureRuntimeRequest, RepositoryCheckout, RuntimeCapabilities, WorkflowExecution,
};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::Request,
};
use tower::ServiceExt;

fn test_state(tmp: &tempfile::TempDir, token: Option<String>) -> AppState {
    AppState {
        config: Arc::new(ManagerConfig {
            runtime_root: tmp.path().join("runtimes"),
            socket_root: tmp.path().join("sockets"),
            api_port_base: 21000,
            api_port_span: 10,
            max_runtimes: 50,
            system_prompt_path: tmp.path().join("system-prompt.txt"),
            seed_config_path: tmp.path().join("config.json"),
            seed_credentials_path: tmp.path().join("credentials.json"),
            mcp_url: None,
            mcp_allowlist: String::new(),
            mcp_token_path: tmp.path().join("mcp-token"),
            kubernetes_namespace: "apps".to_string(),
            pod_image: "ghcr.io/platform-q-ai/quecto:latest".to_string(),
            pod_pull_secret: Some("ghcr-pull-secret".to_string()),
            credentials_secret_name: "quecto-secrets".to_string(),
            manager_self_url: "http://quecto-runtime-manager:8080".to_string(),
            manager_token: token.clone(),
        }),
        registry: Arc::new(Mutex::new(RuntimeRegistry::default())),
        token,
        http: Client::new(),
        lifecycle: Arc::new(ProductionRuntimeLifecycle),
        pending_starts: Arc::new(Mutex::new(HashSet::new())),
    }
}

fn fake_runtime(runtime_ref: &str, socket_path: PathBuf) -> ManagedRuntime {
    ManagedRuntime {
        runtime_ref: runtime_ref.to_string(),
        session_name: "session".to_string(),
        session_key: "key".to_string(),
        base_dir: PathBuf::from("/tmp/runtime"),
        socket_path,
        port: 21000,
        agent: None,
        api: None,
        mcp: None,
        pod_name: None,
        pod_ip: None,
        last_used_at: Instant::now(),
    }
}

#[derive(Default)]
struct FakeLifecycle {
    starts: Arc<Mutex<Vec<(String, String, u16)>>>,
    credential_syncs: Arc<Mutex<Vec<String>>>,
    pod_deletes: Arc<Mutex<Vec<String>>>,
    pod_statuses: Arc<Mutex<Vec<String>>>,
    delay_start: bool,
}

impl RuntimeLifecycle for FakeLifecycle {
    fn start_runtime(
        &self,
        state: AppState,
        body: EnsureRuntimeRequest,
        runtime_ref: String,
        port: u16,
    ) -> BoxFutureResult<ManagedRuntime> {
        let starts = self.starts.clone();
        let delay = self.delay_start;
        Box::pin(async move {
            if delay {
                sleep(Duration::from_millis(50)).await;
            }
            starts.lock().await.push((
                runtime_ref.clone(),
                body.execution_model
                    .clone()
                    .unwrap_or_else(|| "process".to_string()),
                port,
            ));
            let mut runtime = fake_runtime(
                &runtime_ref,
                state.config.socket_root.join(format!("{runtime_ref}.sock")),
            );
            runtime.port = port;
            if body.execution_model.as_deref() == Some("pod") {
                runtime.pod_name = Some(runtime_pod_name(&runtime_ref));
                runtime.pod_ip = Some("10.42.0.10".to_string());
            }
            Ok(runtime)
        })
    }

    fn sync_credentials(&self, _state: AppState, credentials_json: String) -> BoxFutureResult<()> {
        let syncs = self.credential_syncs.clone();
        Box::pin(async move {
            syncs.lock().await.push(credentials_json);
            Ok(())
        })
    }

    fn delete_runtime_pod(&self, _state: AppState, pod_name: String) -> BoxFutureResult<()> {
        let deletes = self.pod_deletes.clone();
        Box::pin(async move {
            deletes.lock().await.push(pod_name);
            Ok(())
        })
    }

    fn runtime_pod_status(&self, _state: AppState, pod_name: String) -> BoxFutureResult<Value> {
        let statuses = self.pod_statuses.clone();
        Box::pin(async move {
            statuses.lock().await.push(pod_name.clone());
            Ok(json!({ "pod_name": pod_name, "phase": "Running" }))
        })
    }
}

fn test_state_with_lifecycle(
    tmp: &tempfile::TempDir,
    token: Option<String>,
    lifecycle: Arc<dyn RuntimeLifecycle>,
) -> AppState {
    let mut state = test_state(tmp, token);
    state.lifecycle = lifecycle;
    state
}

fn board_pod_request() -> EnsureRuntimeRequest {
    EnsureRuntimeRequest {
        agent_profile_id: "jarga-boards".to_string(),
        user_id: None,
        project_id: "board-123".to_string(),
        chat_id: "run-456-card-789".to_string(),
        session_name: "jarga-board-board-123-card-card-789".to_string(),
        session_key: "jarga-boards:board-123:card-789:run-456".to_string(),
        execution_model: Some("pod".to_string()),
        repository: None,
        runtime: None,
        workflow: None,
    }
}

#[test]
fn process_runtime_mcp_registers_community_and_boards_tools() {
    let source = include_str!("infrastructure.rs");

    assert!(source.contains("\"--tool-prefix\",\n                    \"community.\""));
    assert!(source.contains("\"--tool-prefix\",\n                    \"boards.\""));
}

#[test]
fn runtime_pod_manifest_launches_isolated_quecto_api_pod() {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(&tmp, None);
    let config = state.config.as_ref();
    let manifest = runtime_pod_manifest(
        config,
        &board_pod_request(),
        "cc-jarga-boards-board-run",
        "quecto-runtime-cc-jarga-boards-board-run",
    );

    assert_eq!(manifest["kind"], "Pod");
    assert_eq!(manifest["metadata"]["namespace"], "apps");
    assert_eq!(
        manifest["metadata"]["labels"]["managed-by"],
        "quecto-runtime-manager"
    );
    assert_eq!(manifest["spec"]["restartPolicy"], "Never");
    assert_eq!(
        manifest["spec"]["imagePullSecrets"][0]["name"],
        "ghcr-pull-secret"
    );
    assert_eq!(manifest["spec"]["containers"][0]["name"], "quecto");
    assert_eq!(
        manifest["spec"]["containers"][0]["imagePullPolicy"],
        "Always"
    );
    assert_eq!(manifest["spec"]["containers"][1]["name"], "quecto-api");
    assert_eq!(
        manifest["spec"]["containers"][1]["imagePullPolicy"],
        "Always"
    );
    assert_eq!(
        manifest["spec"]["containers"][1]["readinessProbe"]["httpGet"]["path"],
        "/health"
    );
}

#[test]
fn runtime_bootstrap_verifies_toolchain_database_and_project_dependencies_before_agent() {
    let bootstrap = runtime_bootstrap_command();

    assert!(bootstrap.contains("verify_runtime_toolchain"));
    assert!(bootstrap.contains("psql \"$DATABASE_URL\""));
    assert!(bootstrap.contains("setup_project_dependencies"));
    assert!(bootstrap.contains("mix deps.get"));
    assert!(bootstrap.contains("find . -maxdepth 5 -name package.json"));
    assert!(bootstrap.contains("bun install"));
    assert!(bootstrap.contains("npm ci"));
    assert!(bootstrap.contains("npm install"));
    assert!(
        bootstrap
            .contains("setup_project_dependencies\nif [ -n \"${QUECTO_WORKFLOW_CONFIG_JSON:-}\" ]")
    );
    assert!(bootstrap.contains("exec quecto agent"));
}

#[test]
fn runtime_target_url_preserves_query_string_for_incremental_audit_polling() {
    assert_eq!(
        runtime_target_url(
            "http://10.42.0.10:8080",
            "audit/events",
            Some("after=491&limit=500")
        ),
        "http://10.42.0.10:8080/audit/events?after=491&limit=500"
    );
}

#[test]
fn runtime_target_url_omits_empty_query_string() {
    assert_eq!(
        runtime_target_url("http://10.42.0.10:8080", "state", None),
        "http://10.42.0.10:8080/state"
    );
    assert_eq!(
        runtime_target_url("http://10.42.0.10:8080", "state", Some("")),
        "http://10.42.0.10:8080/state"
    );
}

#[test]
fn runtime_pod_manifest_points_api_audit_reader_at_agent_session_log() {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(&tmp, None);
    let config = state.config.as_ref();
    let request = board_pod_request();

    let manifest = runtime_pod_manifest(
        config,
        &request,
        "cc-jarga-boards-board-run",
        "quecto-runtime-cc-jarga-boards-board-run",
    );
    let quecto_api = &manifest["spec"]["containers"][1];
    let env = quecto_api["env"]
        .as_array()
        .expect("env should be an array");
    let session_key = env
        .iter()
        .find(|entry| entry["name"] == "QUECTO_SESSION_KEY")
        .and_then(|entry| entry["value"].as_str())
        .expect("api session key env");

    assert_eq!(
        session_key, "cli:jarga-board-board-123-card-card-789",
        "quecto-api must read the audit JSONL file written by the agent session, not the Boards correlation key"
    );
    assert_ne!(session_key, request.session_key);
}

#[test]
fn runtime_pod_manifest_bootstraps_autonomous_board_workflow_repo() {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(&tmp, None);
    let config = state.config.as_ref();
    let mut request = board_pod_request();
    request.repository = Some(RepositoryCheckout {
        url: "https://github.com/platform-q-ai/quecto.git".to_string(),
        ref_name: Some("master".to_string()),
        working_dir: Some("/home/appuser/workspace/quecto".to_string()),
        auth: Some("github_app".to_string()),
    });
    request.runtime = Some(RuntimeCapabilities {
        network: true,
        sandbox: Some("none".to_string()),
        github_cli: true,
        git_write: true,
    });
    request.workflow = Some(WorkflowExecution {
        config_path: Some("workflow-config.json".to_string()),
        config_json: Some(json!({"workflow": {"templates": []}})),
        template: Some("feature".to_string()),
        stop_after_step_key: Some("reviewers".to_string()),
    });

    let manifest = runtime_pod_manifest(
        config,
        &request,
        "cc-jarga-boards-board-run",
        "quecto-runtime-cc-jarga-boards-board-run",
    );
    let quecto = &manifest["spec"]["containers"][0];
    let env = quecto["env"].as_array().expect("env should be an array");
    let value_for = |name: &str| {
        env.iter()
            .find(|entry| entry["name"] == name)
            .and_then(|entry| entry["value"].as_str())
            .unwrap_or_default()
            .to_string()
    };

    assert_eq!(
        value_for("QUECTO_REPO_URL"),
        "https://github.com/platform-q-ai/quecto.git"
    );
    assert_eq!(value_for("QUECTO_REPO_REF"), "master");
    assert_eq!(
        value_for("QUECTO_WORKDIR"),
        "/home/appuser/workspace/quecto"
    );
    assert_eq!(value_for("QUECTO_WORKFLOW_TEMPLATE"), "feature");
    assert_eq!(value_for("QUECTO_WORKFLOW_STOP_AFTER"), "reviewers");
    let runtime_config: serde_json::Value =
        serde_json::from_str(&value_for("QUECTO_WORKFLOW_CONFIG_JSON")).unwrap();
    assert_eq!(
        runtime_config["tools"]["exec"]["isolation"],
        serde_json::Value::String("native".to_string())
    );
    assert_eq!(
        runtime_config["tools"]["exec"]["allow_native_fallback"],
        serde_json::Value::Bool(true)
    );
    assert_eq!(
        value_for("QUECTO_RUNTIME_CONFIG_PATH"),
        "/home/appuser/.quecto/runtime-configs/cc-jarga-boards-board-run.json"
    );
    assert!(quecto["args"][0].as_str().unwrap().contains("git clone"));
    assert!(
        quecto["args"][0]
            .as_str()
            .unwrap()
            .contains("gh auth login")
    );
    let bootstrap = quecto["args"][0].as_str().unwrap();
    assert!(bootstrap.contains("--config \"$QUECTO_RUNTIME_CONFIG_PATH\""));
    assert!(bootstrap.contains("--workflow --workflow-guards"));
    assert!(bootstrap.contains("/etc/quecto/workflow-agent-system-prompt.txt"));
    assert!(bootstrap.contains(
        "printf '%s' \"$QUECTO_WORKFLOW_CONFIG_JSON\" > \"$QUECTO_RUNTIME_CONFIG_PATH\""
    ));
    assert!(
        bootstrap.contains("cp /home/appuser/.quecto/config.json \"$QUECTO_RUNTIME_CONFIG_PATH\"")
    );
    assert!(env.iter().any(|entry| entry["name"] == "GH_TOKEN"));

    let mounts = quecto["volumeMounts"].as_array().expect("mounts");
    assert!(mounts.iter().any(|mount| {
        mount["mountPath"] == "/etc/quecto/workflow-agent-system-prompt.txt"
            && mount["subPath"] == "workflow-agent-system-prompt.txt"
    }));
    assert!(mounts.iter().any(|mount| {
        mount["mountPath"] == "/etc/quecto/agent-workflow-tools.md"
            && mount["subPath"] == "agent-workflow-tools.md"
    }));

    let prompt_items = manifest["spec"]["volumes"]
        .as_array()
        .expect("volumes")
        .iter()
        .find(|volume| volume["name"] == "prompt")
        .and_then(|volume| volume["configMap"]["items"].as_array())
        .expect("prompt config map items");
    assert!(
        prompt_items
            .iter()
            .any(|item| item["key"] == "workflow-agent-system-prompt.txt")
    );
    assert!(
        prompt_items
            .iter()
            .any(|item| item["key"] == "agent-workflow-tools.md")
    );
}

#[test]
fn runtime_pod_manifest_uses_distinct_runtime_config_paths() {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(&tmp, None);
    let config = state.config.as_ref();
    let request = board_pod_request();

    let first = runtime_pod_manifest(config, &request, "runtime-one", "pod-one");
    let second = runtime_pod_manifest(config, &request, "runtime-two", "pod-two");

    let runtime_config_path = |manifest: &Value| {
        manifest["spec"]["containers"][0]["env"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["name"] == "QUECTO_RUNTIME_CONFIG_PATH")
            .and_then(|entry| entry["value"].as_str())
            .unwrap()
            .to_string()
    };

    assert_eq!(
        runtime_config_path(&first),
        "/home/appuser/.quecto/runtime-configs/runtime-one.json"
    );
    assert_eq!(
        runtime_config_path(&second),
        "/home/appuser/.quecto/runtime-configs/runtime-two.json"
    );
    assert_ne!(runtime_config_path(&first), runtime_config_path(&second));
}

#[tokio::test]
async fn health_reports_active_runtime_count() {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(&tmp, None);

    let response = health(State(state)).await.into_response();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn stop_runtime_requires_auth() {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(&tmp, Some("secret".to_string()));
    let app = router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/runtimes/cc-test")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn sync_credentials_requires_auth() {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(&tmp, Some("secret".to_string()));
    let app = router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/credentials")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"credentials_json":"{}"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn sync_credentials_rejects_invalid_json_payload() {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(&tmp, Some("secret".to_string()));
    let app = router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/credentials")
                .header("authorization", "Bearer secret")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"credentials_json":"not json"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn sync_credentials_requires_credentials_field() {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(&tmp, Some("secret".to_string()));
    let app = router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/credentials")
                .header("authorization", "Bearer secret")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"other":"field"}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[test]
fn runtime_pod_manifest_seeds_credentials_into_writable_volume() {
    let tmp = tempfile::tempdir().unwrap();
    let state = test_state(&tmp, Some("secret".to_string()));
    let config = state.config.as_ref();
    let request = board_pod_request();

    let manifest = runtime_pod_manifest(config, &request, "runtime-one", "pod-one");

    // An init container seeds credentials.json into the writable volume.
    let init = manifest["spec"]["initContainers"]
        .as_array()
        .expect("initContainers")
        .iter()
        .find(|c| c["name"] == "seed-credentials")
        .expect("seed-credentials init container");
    assert!(
        init["args"][0]
            .as_str()
            .unwrap()
            .contains("/home/appuser/.quecto/credentials.json")
    );

    // The main container must NOT mount credentials.json read-only, or the
    // refreshed token could not be persisted.
    let mounts = manifest["spec"]["containers"][0]["volumeMounts"]
        .as_array()
        .expect("volumeMounts");
    assert!(
        !mounts
            .iter()
            .any(|m| m["mountPath"] == "/home/appuser/.quecto/credentials.json"),
        "main container should not mount credentials.json directly (read-only)"
    );

    // The sync callback env is wired with the manager URL and token.
    let env = manifest["spec"]["containers"][0]["env"]
        .as_array()
        .expect("env");
    let env_val = |name: &str| {
        env.iter()
            .find(|e| e["name"] == name)
            .and_then(|e| e["value"].as_str())
            .map(str::to_string)
    };
    assert_eq!(
        env_val("QUECTO_CREDENTIAL_SYNC_URL").as_deref(),
        Some("http://quecto-runtime-manager:8080/credentials")
    );
    assert_eq!(
        env_val("QUECTO_CREDENTIAL_SYNC_TOKEN").as_deref(),
        Some("secret")
    );
}

#[tokio::test]
async fn stop_runtime_is_idempotent_and_removes_socket() {
    let tmp = tempfile::tempdir().unwrap();
    let socket = tmp.path().join("runtime.sock");
    tokio::fs::write(&socket, "stale").await.unwrap();
    let state = test_state(&tmp, Some("secret".to_string()));
    state
        .registry
        .lock()
        .await
        .insert(fake_runtime("cc-test", socket.clone()));
    let app = router(state.clone());

    let response = app
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/runtimes/cc-test")
                .header("authorization", "Bearer secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert!(!socket.exists());
    assert_eq!(state.registry.lock().await.active_count(), 0);
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["stopped"], true);

    let response = router(state)
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/runtimes/cc-test")
                .header("authorization", "Bearer secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["stopped"], false);
}

#[path = "infrastructure_lifecycle_tests.rs"]
mod lifecycle_tests;
