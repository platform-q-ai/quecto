use super::*;
use std::time::Duration;
use tempfile::tempdir;

fn config(root: PathBuf) -> ManagerConfig {
    ManagerConfig {
        runtime_root: root.join("runtimes"),
        socket_root: root.join("sockets"),
        api_port_base: 21000,
        api_port_span: 4,
        max_runtimes: 1,
        system_prompt_path: root.join("system-prompt.txt"),
        seed_config_path: root.join("config.json"),
        seed_credentials_path: root.join("credentials.json"),
        mcp_url: None,
        mcp_allowlist: String::new(),
        mcp_token_path: root.join("mcp-token"),
        kubernetes_namespace: "apps".to_string(),
        pod_image: "ghcr.io/platform-q-ai/quecto:latest".to_string(),
        pod_pull_secret: Some("ghcr-pull-secret".to_string()),
        credentials_secret_name: "quecto-secrets".to_string(),
        manager_self_url: "http://quecto-runtime-manager:8080".to_string(),
        manager_token: None,
    }
}

fn runtime(runtime_ref: &str, port: u16, socket_path: PathBuf) -> ManagedRuntime {
    ManagedRuntime {
        runtime_ref: runtime_ref.to_string(),
        session_name: "session".to_string(),
        session_key: "key".to_string(),
        base_dir: PathBuf::from("/tmp/runtime"),
        socket_path,
        port,
        agent: None,
        api: None,
        mcp: None,
        pod_name: None,
        pod_ip: None,
        last_used_at: Instant::now(),
    }
}

#[test]
fn stop_is_idempotent_and_removes_socket_and_port() {
    let tmp = tempdir().unwrap();
    let socket = tmp.path().join("runtime.sock");
    std::fs::write(&socket, "stale").unwrap();
    let mut registry = RuntimeRegistry::default();
    registry.insert(runtime("cc-test", 21000, socket.clone()));

    assert!(registry.stop("cc-test"));
    assert!(!registry.stop("cc-test"));
    assert!(!socket.exists());
    assert_eq!(registry.active_count(), 0);
    assert!(
        registry
            .allocate_port(&config(tmp.path().to_path_buf()), "cc-test")
            .is_ok()
    );
}

#[test]
fn capacity_reaps_oldest_runtime() {
    let tmp = tempdir().unwrap();
    let cfg = config(tmp.path().to_path_buf());
    let mut registry = RuntimeRegistry::default();
    let mut old = runtime("old", 21000, tmp.path().join("old.sock"));
    old.last_used_at = Instant::now() - Duration::from_secs(120);
    registry.insert(old);

    ensure_capacity(&mut registry, &cfg, "new").unwrap();

    assert_eq!(registry.active_count(), 0);
}
